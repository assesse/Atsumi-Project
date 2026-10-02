//! Private bookmarks; deliberately independent from discovery favorites and public reviews.
use std::{collections::HashMap, path::Path, time::Duration};

use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

const SCHEMA: &str = r#"
CREATE TABLE library_meta(singleton INTEGER PRIMARY KEY CHECK(singleton=1), revision INTEGER NOT NULL);
INSERT INTO library_meta VALUES(1,0);
CREATE TABLE bookmarks(
 source TEXT NOT NULL CHECK(source='hitomi'), gallery_id INTEGER NOT NULL CHECK(gallery_id>0),
 page INTEGER NOT NULL CHECK(page>=0), snapshot TEXT NOT NULL CHECK(json_valid(snapshot)),
 search_text TEXT NOT NULL, page_sha256 TEXT, saved_entry TEXT, saved_revision INTEGER,
 created_at TEXT NOT NULL, PRIMARY KEY(source,gallery_id,page)
) STRICT;
CREATE INDEX bookmarks_recent ON bookmarks(created_at DESC,gallery_id DESC,page DESC);
CREATE TABLE collections(id TEXT PRIMARY KEY, name TEXT NOT NULL, name_key TEXT NOT NULL UNIQUE) STRICT;
CREATE TABLE collection_items(
 collection_id TEXT NOT NULL REFERENCES collections(id) ON DELETE CASCADE,
 source TEXT NOT NULL, gallery_id INTEGER NOT NULL, page INTEGER NOT NULL,
 PRIMARY KEY(collection_id,source,gallery_id,page),
 FOREIGN KEY(source,gallery_id,page) REFERENCES bookmarks(source,gallery_id,page) ON DELETE CASCADE
) STRICT;
CREATE INDEX collection_items_target ON collection_items(source,gallery_id,page);
PRAGMA user_version=1;
"#;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Target {
    pub source: String,
    pub gallery_id: i64,
    pub page: u32,
}

impl Target {
    fn validate(&self) -> Result<(), String> {
        if self.source != "hitomi"
            || self.gallery_id <= 0
            || self.gallery_id > 9_007_199_254_740_991
            || self.page > 100_000
        {
            return Err("지원하지 않는 즐겨찾기 대상입니다.".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GallerySnapshot {
    pub title: String,
    pub artist: String,
    pub artists: Vec<String>,
    pub pages: u32,
    pub language: String,
    pub thumbnail_width: Option<u32>,
    pub thumbnail_height: Option<u32>,
}

impl GallerySnapshot {
    fn validate(&self, page: u32) -> Result<(), String> {
        if self.title.trim().is_empty()
            || self.title.len() > 8192
            || self.artist.len() > 2048
            || self.artists.len() > 200
            || self.artists.iter().any(|s| s.len() > 2048)
            || self.pages == 0
            || self.pages > 100_000
            || page > self.pages
            || self.language.len() > 32
        {
            return Err("앨범 정보를 확인한 후 다시 저장해 주세요.".into());
        }
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(
    tag = "action",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum Request {
    Summary {},
    List {
        kind: String,
        collection_id: Option<String>,
        search: String,
        offset: u32,
        limit: u32,
    },
    Get {
        target: Target,
    },
    BookmarkSet {
        target: Target,
        enabled: bool,
        snapshot: Option<GallerySnapshot>,
    },
    CollectionSave {
        id: Option<String>,
        name: String,
    },
    CollectionDelete {
        id: String,
    },
    MembershipSet {
        target: Target,
        collection_id: String,
        enabled: bool,
    },
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedKey {
    #[serde(flatten)]
    target: Target,
    collection_ids: Vec<String>,
}
#[derive(Serialize)]
pub struct Collection {
    id: String,
    name: String,
    count: u32,
}
#[derive(Serialize)]
pub struct Summary {
    revision: i64,
    keys: Vec<SavedKey>,
    collections: Vec<Collection>,
}
#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Reference {
    status: String,
    entry_id: Option<String>,
    #[serde(skip)]
    sha256: Option<String>,
    #[serde(skip)]
    revision: Option<i64>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Bookmark {
    #[serde(flatten)]
    target: Target,
    snapshot: GallerySnapshot,
    created_at: String,
    reference: Reference,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Response {
    summary: Summary,
    items: Vec<Bookmark>,
    total: u32,
    collection_id: Option<String>,
}

fn db_error(e: impl std::fmt::Display) -> String {
    format!("개인 즐겨찾기를 처리하지 못했습니다: {e}")
}

fn open_library(path: &Path) -> Result<Connection, String> {
    let mut c = Connection::open(path).map_err(db_error)?;
    c.busy_timeout(Duration::from_secs(3)).map_err(db_error)?;
    c.execute_batch("PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;")
        .map_err(db_error)?;
    // The version check is inside the writer transaction, including concurrent first use.
    let tx = c
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(db_error)?;
    let version: i64 = tx
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .map_err(db_error)?;
    match version {
        0 => tx.execute_batch(SCHEMA).map_err(db_error)?,
        1 => {}
        _ => {
            return Err(
                "더 최신 버전에서 저장된 개인 즐겨찾기입니다. 앱을 업데이트해 주세요.".into(),
            )
        }
    }
    tx.commit().map_err(db_error)?;
    Ok(c)
}

fn summary(c: &Connection) -> Result<Summary, String> {
    let revision = c
        .query_row("SELECT revision FROM library_meta", [], |r| r.get(0))
        .map_err(db_error)?;
    let mut keys = Vec::new();
    let mut statement = c
        .prepare("SELECT source,gallery_id,page FROM bookmarks ORDER BY gallery_id,page")
        .map_err(db_error)?;
    let rows = statement
        .query_map([], |r| {
            Ok(Target {
                source: r.get(0)?,
                gallery_id: r.get(1)?,
                page: r.get(2)?,
            })
        })
        .map_err(db_error)?;
    let mut memberships: HashMap<(String, i64, u32), Vec<String>> = HashMap::new();
    let mut membership = c.prepare("SELECT source,gallery_id,page,collection_id FROM collection_items ORDER BY collection_id").map_err(db_error)?;
    let links = membership
        .query_map([], |r| {
            Ok((
                (
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, u32>(2)?,
                ),
                r.get::<_, String>(3)?,
            ))
        })
        .map_err(db_error)?;
    for link in links {
        let (target, collection_id) = link.map_err(db_error)?;
        memberships.entry(target).or_default().push(collection_id);
    }
    for row in rows {
        let target = row.map_err(db_error)?;
        let collection_ids = memberships
            .remove(&(target.source.clone(), target.gallery_id, target.page))
            .unwrap_or_default();
        keys.push(SavedKey {
            target,
            collection_ids,
        });
    }
    let collections = c.prepare("SELECT c.id,c.name,COUNT(i.collection_id) FROM collections c LEFT JOIN collection_items i ON i.collection_id=c.id GROUP BY c.id ORDER BY c.name_key,c.id").map_err(db_error)?
        .query_map([], |r| Ok(Collection { id:r.get(0)?,name:r.get(1)?,count:r.get(2)? })).map_err(db_error)?
        .collect::<rusqlite::Result<Vec<_>>>().map_err(db_error)?;
    Ok(Summary {
        revision,
        keys,
        collections,
    })
}

// Read only database identity and state; never read/hash user images or repair exclusions.
fn reference(c: Option<&Connection>, target: &Target) -> Result<Reference, String> {
    let Some(c) = c else {
        return Ok(Reference {
            status: "remote".into(),
            ..Default::default()
        });
    };
    let excluded: bool = c.query_row("SELECT EXISTS(SELECT 1 FROM duplicate_hidden_galleries h WHERE gallery_id=?1 AND NOT EXISTS(SELECT 1 FROM exploration_restored_galleries WHERE gallery_id=?1))", [target.gallery_id], |r| r.get(0)).map_err(db_error)?;
    if excluded {
        return Ok(Reference {
            status: "excluded".into(),
            ..Default::default()
        });
    }
    let entry: Option<(String,String,Option<String>,Option<i64>)> = c.query_row(
        "SELECT e.entry_id,e.state,a.state,a.revision FROM download_entries e LEFT JOIN download_artifacts a ON a.entry_id=e.entry_id WHERE e.gallery_id=?1 ORDER BY e.created_at DESC,e.entry_id DESC LIMIT 1",
        [target.gallery_id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(db_error)?;
    let Some((id, state, artifact_state, revision)) = entry else {
        return Ok(Reference {
            status: "remote".into(),
            ..Default::default()
        });
    };
    if state == "quarantined" || artifact_state.as_deref() == Some("quarantined") {
        return Ok(Reference {
            status: "excluded".into(),
            ..Default::default()
        });
    }
    let relocating: bool = c.query_row("SELECT EXISTS(SELECT 1 FROM excluded_artifact_relocations WHERE entry_id=?1 AND state<>'restored') OR EXISTS(SELECT 1 FROM overlap_page_merges WHERE state NOT IN('applied','rolled_back') AND ?1 IN(source_entry_id,target_entry_id))", [&id], |r| r.get(0)).map_err(db_error)?;
    if relocating {
        return Ok(Reference {
            status: "unavailable".into(),
            ..Default::default()
        });
    }
    if state != "completed" {
        return Ok(Reference {
            status: "remote".into(),
            ..Default::default()
        });
    }
    if artifact_state.as_deref() != Some("complete") {
        return Ok(Reference {
            status: "unavailable".into(),
            ..Default::default()
        });
    }
    let mut result = Reference {
        status: "local".into(),
        entry_id: Some(id.clone()),
        revision,
        sha256: None,
    };
    if target.page > 0 {
        let page: Option<(String,bool,Option<String>)> = c.query_row("SELECT state,excluded,sha256 FROM download_pages WHERE entry_id=?1 AND source_page_number=?2", params![id,target.page], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(db_error)?;
        match page {
            Some((state, false, Some(hash))) if state == "present" => result.sha256 = Some(hash),
            Some((_, true, _)) => result.status = "excluded".into(),
            _ => result.status = "unavailable".into(),
        }
    }
    Ok(result)
}

fn read_item(
    c: &Connection,
    main: Option<&Connection>,
    target: Target,
) -> Result<Bookmark, String> {
    let (json,date,hash,entry,revision): (String,String,Option<String>,Option<String>,Option<i64>) = c.query_row(
        "SELECT snapshot,created_at,page_sha256,saved_entry,saved_revision FROM bookmarks WHERE source=?1 AND gallery_id=?2 AND page=?3",
        params![target.source,target.gallery_id,target.page], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).map_err(db_error)?;
    let snapshot = serde_json::from_str(&json).map_err(db_error)?;
    let mut current = reference(main, &target)?;
    if target.page > 0 && matches!(current.status.as_str(), "remote" | "local") {
        if (hash.is_some() && (current.sha256 != hash || current.entry_id.is_none()))
            || (hash.is_none()
                && entry.is_some()
                && (current.entry_id != entry || current.revision != revision))
        {
            current.status = "changed".into();
        } else if hash.is_none() {
            if let (Some(main), Some(id)) = (main, current.entry_id.as_ref()) {
                let merged: bool = main.query_row("SELECT EXISTS(SELECT 1 FROM overlap_page_merges WHERE target_entry_id=?1 AND state='applied')", [id], |r| r.get(0)).map_err(db_error)?;
                if merged {
                    current.status = "changed".into();
                }
            }
        }
    }
    Ok(Bookmark {
        target,
        snapshot,
        created_at: date,
        reference: current,
    })
}

fn run(
    c: &mut Connection,
    main: Option<&Connection>,
    request: Request,
) -> Result<Response, String> {
    let mut items = Vec::new();
    let mut total = 0;
    let mut collection_id = None;
    match request {
        Request::Summary {} => {}
        Request::List {
            kind,
            collection_id: filter,
            search,
            offset,
            limit,
        } => {
            if !matches!(kind.as_str(), "albums" | "pages")
                || !(1..=100).contains(&limit)
                || search.len() > 1000
            {
                return Err("목록 조회 조건이 올바르지 않습니다.".into());
            }
            let predicate = "FROM bookmarks b WHERE ((?1='albums' AND b.page=0) OR (?1='pages' AND b.page>0)) AND (?2 IS NULL OR (?2='unfiled' AND NOT EXISTS(SELECT 1 FROM collection_items i WHERE i.source=b.source AND i.gallery_id=b.gallery_id AND i.page=b.page)) OR EXISTS(SELECT 1 FROM collection_items i WHERE i.collection_id=?2 AND i.source=b.source AND i.gallery_id=b.gallery_id AND i.page=b.page)) AND instr(b.search_text,?3)>0";
            let search = search.trim().to_lowercase();
            total = c
                .query_row(
                    &format!("SELECT COUNT(*) {predicate}"),
                    params![kind, filter, search],
                    |r| r.get(0),
                )
                .map_err(db_error)?;
            let targets = c.prepare(&format!("SELECT b.source,b.gallery_id,b.page {predicate} ORDER BY b.created_at DESC,b.gallery_id DESC,b.page DESC LIMIT ?4 OFFSET ?5")).map_err(db_error)?
                .query_map(params![kind,filter,search,limit,offset], |r| Ok(Target { source:r.get(0)?,gallery_id:r.get(1)?,page:r.get(2)? })).map_err(db_error)?
                .collect::<rusqlite::Result<Vec<_>>>().map_err(db_error)?;
            for target in targets {
                items.push(read_item(c, main, target)?);
            }
        }
        Request::Get { target } => {
            target.validate()?;
            items.push(read_item(c, main, target)?);
            total = 1;
        }
        request => {
            let tx = c
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .map_err(db_error)?;
            match request {
                Request::BookmarkSet {
                    target,
                    enabled,
                    snapshot,
                } => {
                    target.validate()?;
                    if enabled {
                        let snapshot = snapshot.ok_or("앨범 정보가 없습니다.")?;
                        snapshot.validate(target.page)?;
                        let current = reference(main, &target)?;
                        let text = format!(
                            "{} {} {} {}",
                            snapshot.title,
                            snapshot.artist,
                            snapshot.artists.join(" "),
                            target.gallery_id
                        )
                        .to_lowercase();
                        tx.execute("INSERT INTO bookmarks(source,gallery_id,page,snapshot,search_text,page_sha256,saved_entry,saved_revision,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,strftime('%Y-%m-%dT%H:%M:%fZ','now')) ON CONFLICT(source,gallery_id,page) DO NOTHING",
                            params![target.source,target.gallery_id,target.page,serde_json::to_string(&snapshot).map_err(db_error)?,text,current.sha256,current.entry_id,current.revision]).map_err(db_error)?;
                    } else {
                        tx.execute(
                            "DELETE FROM bookmarks WHERE source=?1 AND gallery_id=?2 AND page=?3",
                            params![target.source, target.gallery_id, target.page],
                        )
                        .map_err(db_error)?;
                    }
                }
                Request::CollectionSave { id, name } => {
                    let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
                    if name.is_empty()
                        || name.chars().count() > 60
                        || name.chars().any(char::is_control)
                    {
                        return Err("컬렉션 이름은 1~60자로 입력해 주세요.".into());
                    }
                    let key = name.to_lowercase();
                    let duplicate: Option<String> = tx
                        .query_row(
                            "SELECT id FROM collections WHERE name_key=?1",
                            [&key],
                            |r| r.get(0),
                        )
                        .optional()
                        .map_err(db_error)?;
                    if duplicate.is_some() && duplicate.as_ref() != id.as_ref() {
                        return Err("같은 이름의 컬렉션이 있습니다.".into());
                    }
                    let new_id = id
                        .clone()
                        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
                    let changed = if id.is_some() {
                        tx.execute(
                            "UPDATE collections SET name=?1,name_key=?2 WHERE id=?3",
                            params![name, key, new_id],
                        )
                    } else {
                        tx.execute(
                            "INSERT INTO collections(id,name,name_key) VALUES(?3,?1,?2)",
                            params![name, key, new_id],
                        )
                    }
                    .map_err(db_error)?;
                    if changed != 1 {
                        return Err("컬렉션을 찾을 수 없습니다.".into());
                    }
                    collection_id = Some(new_id);
                }
                Request::CollectionDelete { id } => {
                    tx.execute("DELETE FROM collections WHERE id=?1", [id])
                        .map_err(db_error)?;
                }
                Request::MembershipSet {
                    target,
                    collection_id,
                    enabled,
                } => {
                    target.validate()?;
                    if enabled {
                        tx.execute("INSERT OR IGNORE INTO collection_items(collection_id,source,gallery_id,page) VALUES(?1,?2,?3,?4)",params![collection_id,target.source,target.gallery_id,target.page]).map_err(db_error)?;
                    } else {
                        tx.execute("DELETE FROM collection_items WHERE collection_id=?1 AND source=?2 AND gallery_id=?3 AND page=?4",params![collection_id,target.source,target.gallery_id,target.page]).map_err(db_error)?;
                    }
                }
                _ => unreachable!(),
            }
            tx.execute("UPDATE library_meta SET revision=revision+1", [])
                .map_err(db_error)?;
            tx.commit().map_err(db_error)?;
        }
    }
    Ok(Response {
        summary: summary(c)?,
        items,
        total,
        collection_id,
    })
}

fn execute(data_dir: &Path, request: Request) -> Result<Response, String> {
    std::fs::create_dir_all(data_dir).map_err(db_error)?;
    let mut c = open_library(&data_dir.join("personal-library.sqlite3"))?;
    let needs_main = matches!(
        request,
        Request::List { .. } | Request::Get { .. } | Request::BookmarkSet { enabled: true, .. }
    );
    let main_path = data_dir.join("atsumi-next.sqlite3");
    let main = if needs_main && main_path.exists() {
        let db = Connection::open_with_flags(
            main_path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(db_error)?;
        db.busy_timeout(Duration::from_secs(2)).map_err(db_error)?;
        Some(db)
    } else {
        None
    };
    run(&mut c, main.as_ref(), request)
}

#[tauri::command]
pub async fn personal_library(app: AppHandle, request: Request) -> Result<Response, String> {
    let data_dir = app.path().app_data_dir().map_err(db_error)?;
    tauri::async_runtime::spawn_blocking(move || execute(&data_dir, request))
        .await
        .map_err(db_error)?
}

#[cfg(test)]
mod tests {
    use super::*;
    fn target(page: u32) -> Target {
        Target {
            source: "hitomi".into(),
            gallery_id: 123,
            page,
        }
    }
    fn snapshot() -> GallerySnapshot {
        GallerySnapshot {
            title: "기억할 작품".into(),
            artist: "artist".into(),
            artists: vec!["artist".into()],
            pages: 20,
            language: "korean".into(),
            thumbnail_width: None,
            thumbnail_height: None,
        }
    }
    fn save(page: u32) -> Request {
        Request::BookmarkSet {
            target: target(page),
            enabled: true,
            snapshot: Some(snapshot()),
        }
    }
    fn list(kind: &str, filter: Option<String>) -> Request {
        Request::List {
            kind: kind.into(),
            collection_id: filter,
            search: "".into(),
            offset: 0,
            limit: 50,
        }
    }
    fn memory() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        c.execute_batch(SCHEMA).unwrap();
        c
    }
    #[test]
    fn album_pages_collections_and_removal_are_independent_and_durable() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("personal-library.sqlite3");
        let mut c = open_library(&path).unwrap();
        run(&mut c, None, save(0)).unwrap();
        run(&mut c, None, save(7)).unwrap();
        run(&mut c, None, save(8)).unwrap();
        run(&mut c, None, save(7)).unwrap();
        assert_eq!(summary(&c).unwrap().keys.len(), 3);
        let first = run(
            &mut c,
            None,
            Request::CollectionSave {
                id: None,
                name: " 다시 볼 작품 ".into(),
            },
        )
        .unwrap()
        .collection_id
        .unwrap();
        let second = run(
            &mut c,
            None,
            Request::CollectionSave {
                id: None,
                name: "장면".into(),
            },
        )
        .unwrap()
        .collection_id
        .unwrap();
        for id in [&first, &second] {
            run(
                &mut c,
                None,
                Request::MembershipSet {
                    target: target(7),
                    collection_id: id.clone(),
                    enabled: true,
                },
            )
            .unwrap();
        }
        assert_eq!(
            run(&mut c, None, list("pages", Some(first.clone())))
                .unwrap()
                .total,
            1
        );
        assert_eq!(
            run(&mut c, None, list("pages", Some("unfiled".into())))
                .unwrap()
                .total,
            1
        );
        run(
            &mut c,
            None,
            Request::CollectionSave {
                id: Some(first.clone()),
                name: "모아두기".into(),
            },
        )
        .unwrap();
        run(&mut c, None, Request::CollectionDelete { id: first }).unwrap();
        assert_eq!(summary(&c).unwrap().keys.len(), 3);
        drop(c);
        let mut c = open_library(&path).unwrap();
        assert_eq!(run(&mut c, None, list("albums", None)).unwrap().total, 1);
        assert_eq!(
            run(&mut c, None, list("pages", Some(second)))
                .unwrap()
                .total,
            1
        );
        run(
            &mut c,
            None,
            Request::BookmarkSet {
                target: target(0),
                enabled: false,
                snapshot: None,
            },
        )
        .unwrap();
        assert_eq!(summary(&c).unwrap().keys.len(), 2);
    }
    #[test]
    fn rejects_invalid_targets_duplicate_names_and_dangling_membership_without_partial_writes() {
        let mut c = memory();
        assert!(run(&mut c, None, save(21)).is_err());
        assert!(run(
            &mut c,
            None,
            Request::BookmarkSet {
                target: Target {
                    source: "other".into(),
                    ..target(0)
                },
                enabled: true,
                snapshot: Some(snapshot())
            }
        )
        .is_err());
        assert!(run(
            &mut c,
            None,
            Request::CollectionSave {
                id: None,
                name: " ".into()
            }
        )
        .is_err());
        run(
            &mut c,
            None,
            Request::CollectionSave {
                id: None,
                name: "Scenes".into(),
            },
        )
        .unwrap();
        assert!(run(
            &mut c,
            None,
            Request::CollectionSave {
                id: None,
                name: "SCENES".into()
            }
        )
        .is_err());
        assert!(run(
            &mut c,
            None,
            Request::MembershipSet {
                target: target(7),
                collection_id: "missing".into(),
                enabled: true
            }
        )
        .is_err());
        assert_eq!(summary(&c).unwrap().revision, 1);
        assert!(summary(&c).unwrap().keys.is_empty());
    }
    fn source() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch("CREATE TABLE duplicate_hidden_galleries(gallery_id INTEGER); CREATE TABLE exploration_restored_galleries(gallery_id INTEGER); CREATE TABLE download_entries(entry_id TEXT,gallery_id INTEGER,state TEXT,created_at TEXT); CREATE TABLE download_artifacts(entry_id TEXT,state TEXT,revision INTEGER); CREATE TABLE download_pages(entry_id TEXT,source_page_number INTEGER,state TEXT,excluded INTEGER,sha256 TEXT); CREATE TABLE excluded_artifact_relocations(entry_id TEXT,state TEXT); CREATE TABLE overlap_page_merges(source_entry_id TEXT,target_entry_id TEXT,state TEXT); INSERT INTO download_entries VALUES('entry',123,'completed','now'); INSERT INTO download_artifacts VALUES('entry','complete',1); INSERT INTO download_pages VALUES('entry',7,'present',0,'original-hash');").unwrap();
        c
    }
    #[test]
    fn changed_or_excluded_pages_never_silently_open_different_content() {
        let mut c = memory();
        let main = source();
        run(&mut c, Some(&main), save(7)).unwrap();
        assert_eq!(
            run(&mut c, Some(&main), list("pages", None)).unwrap().items[0]
                .reference
                .status,
            "local"
        );
        main.execute("UPDATE download_pages SET sha256='replacement'", [])
            .unwrap();
        assert_eq!(
            run(&mut c, Some(&main), Request::Get { target: target(7) })
                .unwrap()
                .items[0]
                .reference
                .status,
            "changed"
        );
        main.execute("INSERT INTO duplicate_hidden_galleries VALUES(123)", [])
            .unwrap();
        assert_eq!(
            run(&mut c, Some(&main), list("pages", None)).unwrap().items[0]
                .reference
                .status,
            "excluded"
        );
        let hidden: i64 = main
            .query_row("SELECT COUNT(*) FROM duplicate_hidden_galleries", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(hidden, 1);
        assert_eq!(summary(&c).unwrap().keys.len(), 1);
    }
    #[test]
    fn snapshots_survive_source_reset_and_lists_are_bounded_and_searchable() {
        let mut c = memory();
        let main = source();
        run(&mut c, Some(&main), save(0)).unwrap();
        run(&mut c, Some(&main), save(7)).unwrap();
        let found = run(
            &mut c,
            None,
            Request::List {
                kind: "albums".into(),
                collection_id: None,
                search: "기억".into(),
                offset: 0,
                limit: 1,
            },
        )
        .unwrap();
        assert_eq!(found.total, 1);
        assert_eq!(found.items.len(), 1);
        assert_eq!(
            run(&mut c, None, list("pages", None)).unwrap().items[0]
                .reference
                .status,
            "changed"
        );
        assert!(run(
            &mut c,
            None,
            Request::List {
                kind: "pages".into(),
                collection_id: None,
                search: "".into(),
                offset: 0,
                limit: 10000
            }
        )
        .is_err());
    }
    #[test]
    fn future_personal_schema_is_rejected_without_resetting_it() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("personal-library.sqlite3");
        let c = open_library(&path).unwrap();
        c.execute_batch("PRAGMA user_version=2;").unwrap();
        drop(c);
        assert!(open_library(&path).is_err());
        let c = Connection::open(path).unwrap();
        let version: i64 = c
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, 2);
    }

    #[test]
    fn works_with_the_real_source_schema_and_survives_factory_reset() {
        let folder = tempfile::tempdir().unwrap();
        let main = folder.path().join("atsumi-next.sqlite3");
        drop(crate::infrastructure::SqliteRepository::open(&main).unwrap());
        let source_before = std::fs::read(&main).unwrap();
        execute(folder.path(), save(0)).unwrap();
        execute(folder.path(), save(7)).unwrap();
        let collection = execute(
            folder.path(),
            Request::CollectionSave {
                id: None,
                name: "보존할 장면".into(),
            },
        )
        .unwrap()
        .collection_id
        .unwrap();
        execute(
            folder.path(),
            Request::MembershipSet {
                target: target(7),
                collection_id: collection.clone(),
                enabled: true,
            },
        )
        .unwrap();
        assert_eq!(
            execute(folder.path(), list("pages", Some(collection.clone())))
                .unwrap()
                .items[0]
                .reference
                .status,
            "remote"
        );
        assert_eq!(std::fs::read(&main).unwrap(), source_before);
        std::fs::write(folder.path().join("factory-reset.pending"), b"v1\n").unwrap();
        crate::apply_pending_factory_reset(folder.path()).unwrap();
        assert!(!main.exists());
        let result = execute(folder.path(), list("pages", Some(collection))).unwrap();
        assert_eq!(result.total, 1);
        assert_eq!(result.summary.keys.len(), 2);
        assert_eq!(result.items[0].snapshot.title, "기억할 작품");
    }

    #[test]
    fn ipc_contract_uses_camel_case_fields_and_never_exposes_local_hashes() {
        let request: Request = serde_json::from_value(serde_json::json!({
            "action": "list", "kind": "pages", "collectionId": null, "search": "", "offset": 0, "limit": 20
        })).unwrap();
        let mut c = memory();
        run(&mut c, Some(&source()), save(7)).unwrap();
        let response =
            serde_json::to_value(run(&mut c, Some(&source()), request).unwrap()).unwrap();
        assert_eq!(response["items"][0]["galleryId"], 123);
        assert_eq!(response["items"][0]["reference"]["entryId"], "entry");
        assert!(response["items"][0]["reference"].get("sha256").is_none());
        assert!(response["summary"]["keys"][0]["collectionIds"].is_array());
        assert!(serde_json::from_value::<Request>(
            serde_json::json!({"action":"summary","extra":true})
        )
        .is_err());
    }
}
