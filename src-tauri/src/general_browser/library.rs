//! Browser-owned browsing data. SQLite work is serialized away from the UI thread.
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::{
    path::Path,
    sync::mpsc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const PAGE_SIZE: usize = 50;
const HISTORY_LIMIT: usize = 20_000;
type Job = Box<dyn FnOnce(&mut Connection) + Send>;
#[derive(Clone)]
pub struct Store {
    sender: mpsc::Sender<Job>,
}
#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub id: String,
    pub url: String,
    pub title: String,
    pub visited_at: i64,
}
#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Page {
    pub items: Vec<Entry>,
    pub has_more: bool,
    pub next_offset: usize,
}
fn database_error(_: rusqlite::Error) -> String {
    "방문 기록을 처리하지 못했습니다. 저장 공간을 확인해 주세요.".into()
}
fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}
fn initialize(c: &Connection) -> rusqlite::Result<()> {
    c.busy_timeout(Duration::from_secs(2))?;
    c.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;
      CREATE TABLE IF NOT EXISTS browser_meta(key TEXT PRIMARY KEY,value TEXT NOT NULL);
      CREATE TABLE IF NOT EXISTS visits(id TEXT PRIMARY KEY,url TEXT NOT NULL,title TEXT NOT NULL,visited_at INTEGER NOT NULL);
      CREATE INDEX IF NOT EXISTS visits_time ON visits(visited_at DESC,id DESC);
      CREATE INDEX IF NOT EXISTS visits_url ON visits(url);
      CREATE TABLE IF NOT EXISTS bookmarks(url TEXT PRIMARY KEY,title TEXT NOT NULL,created_at INTEGER NOT NULL);")
}
/// Never put credentials from login callbacks or URL fragments into the history index.
pub fn clean_url(input: &str) -> Option<String> {
    if input.len() > 8192 {
        return None;
    }
    let mut url = tauri::Url::parse(input).ok()?;
    if !super::allowed(&url) || !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    url.set_fragment(None);
    let pairs: Vec<_> = url
        .query_pairs()
        .filter(|(key, _)| {
            !matches!(
                key.to_ascii_lowercase().as_str(),
                "code"
                    | "token"
                    | "access_token"
                    | "id_token"
                    | "refresh_token"
                    | "password"
                    | "passwd"
                    | "secret"
                    | "authorization"
                    | "api_key"
                    | "apikey"
                    | "session"
                    | "sessionid"
            )
        })
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    url.set_query(None);
    if !pairs.is_empty() {
        url.query_pairs_mut().extend_pairs(pairs);
    }
    Some(url.into())
}
fn import_previous_history(c: &mut Connection, root: &Path) -> rusqlite::Result<()> {
    if c.query_row(
        "SELECT EXISTS(SELECT 1 FROM browser_meta WHERE key='webview-history-imported')",
        [],
        |r| r.get::<_, bool>(0),
    )? {
        return Ok(());
    }
    let path = root.join("web-profile/EBWebView/Default/History");
    let tx = c.transaction()?;
    if path.exists() {
        let source = Connection::open_with_flags(
            path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        source.busy_timeout(Duration::from_millis(500))?;
        source.execute_batch("PRAGMA query_only=ON;")?;
        let mut query=source.prepare("SELECT v.id,u.url,u.title,v.visit_time FROM visits v JOIN urls u ON u.id=v.url WHERE u.hidden=0 ORDER BY v.visit_time DESC LIMIT 20000")?;
        for row in query.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
            ))
        })? {
            let (id, url, title, webkit_time) = row?;
            if let Some(url) = clean_url(&url) {
                let at = webkit_time / 1000 - 11_644_473_600_000i64;
                if at > 0 {
                    tx.execute(
                        "INSERT OR IGNORE INTO visits VALUES(?1,?2,?3,?4)",
                        params![
                            format!("webview-{id}"),
                            url,
                            title.chars().take(160).collect::<String>(),
                            at
                        ],
                    )?;
                }
            }
        }
    }
    tx.execute(
        "INSERT INTO browser_meta VALUES('webview-history-imported','1')",
        [],
    )?;
    tx.commit()
}
fn record(c: &Connection, url: &str, title: &str, at: i64) -> rusqlite::Result<()> {
    c.execute(
        "INSERT INTO visits VALUES(?1,?2,?3,?4)",
        params![
            uuid::Uuid::new_v4().simple().to_string(),
            url,
            title.chars().take(160).collect::<String>(),
            at
        ],
    )?;
    c.execute("DELETE FROM visits WHERE id IN (SELECT id FROM visits ORDER BY visited_at DESC,id DESC LIMIT -1 OFFSET ?1)",[HISTORY_LIMIT as i64])?;
    Ok(())
}
fn query(c: &Connection, kind: &str, text: &str, offset: usize) -> Result<Page, String> {
    let offset = offset.min(HISTORY_LIMIT);
    let search = format!(
        "%{}%",
        text.chars()
            .take(200)
            .collect::<String>()
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_")
    );
    let sql=match kind {
        "history"=>"SELECT id,url,title,visited_at FROM visits WHERE title LIKE ?1 ESCAPE '\\' OR url LIKE ?1 ESCAPE '\\' ORDER BY visited_at DESC,id DESC LIMIT ?2 OFFSET ?3",
        "bookmarks"=>"SELECT url,url,title,created_at FROM bookmarks WHERE title LIKE ?1 ESCAPE '\\' OR url LIKE ?1 ESCAPE '\\' ORDER BY created_at DESC,url DESC LIMIT ?2 OFFSET ?3",
        _=>return Err("목록을 찾지 못했습니다.".into())
    };
    let mut stmt = c.prepare(sql).map_err(database_error)?;
    let mut items = stmt
        .query_map(
            params![search, (PAGE_SIZE + 1) as i64, offset as i64],
            |r| {
                Ok(Entry {
                    id: r.get(0)?,
                    url: r.get(1)?,
                    title: r.get(2)?,
                    visited_at: r.get(3)?,
                })
            },
        )
        .map_err(database_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(database_error)?;
    let has_more = items.len() > PAGE_SIZE;
    items.truncate(PAGE_SIZE);
    Ok(Page {
        next_offset: offset + items.len(),
        items,
        has_more,
    })
}
fn edit(
    c: &Connection,
    kind: &str,
    action: &str,
    id: &str,
    url: &str,
    title: &str,
) -> Result<bool, String> {
    match (kind, action) {
        ("history", "remove") => {
            c.execute("DELETE FROM visits WHERE id=?1", [id])
                .map_err(database_error)?;
        }
        ("history", "clear") => {
            c.execute("DELETE FROM visits", [])
                .map_err(database_error)?;
        }
        ("bookmarks", "remove") => {
            c.execute(
                "DELETE FROM bookmarks WHERE url=?1",
                [clean_url(id).unwrap_or_default()],
            )
            .map_err(database_error)?;
        }
        ("bookmarks", "status") => {
            return c
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM bookmarks WHERE url=?1)",
                    [clean_url(url).unwrap_or_default()],
                    |r| r.get(0),
                )
                .map_err(database_error)
        }
        ("bookmarks", "add") => {
            let url = clean_url(url).ok_or("이 페이지는 북마크에 저장할 수 없습니다.")?;
            c.execute("INSERT INTO bookmarks VALUES(?1,?2,?3) ON CONFLICT(url) DO UPDATE SET title=excluded.title",params![url,title.chars().take(160).collect::<String>(),now()]).map_err(database_error)?;
            return Ok(true);
        }
        _ => return Err("지원하지 않는 목록 동작입니다.".into()),
    }
    Ok(false)
}
impl Store {
    pub fn open(root: &Path) -> Result<Self, String> {
        let mut c = Connection::open(root.join("library.sqlite3")).map_err(database_error)?;
        initialize(&c).map_err(database_error)?;
        let (sender, receiver) = mpsc::channel::<Job>();
        let root = root.to_owned();
        std::thread::Builder::new()
            .name("atsumi-browser-library".into())
            .spawn(move || {
                let _ = import_previous_history(&mut c, &root);
                while let Ok(job) = receiver.recv() {
                    job(&mut c);
                }
            })
            .map_err(|_| "방문 기록을 준비하지 못했습니다.")?;
        Ok(Self { sender })
    }
    async fn run<T: Send + 'static>(
        &self,
        work: impl FnOnce(&mut Connection) -> Result<T, String> + Send + 'static,
    ) -> Result<T, String> {
        let (send, receive) = tokio::sync::oneshot::channel();
        self.sender
            .send(Box::new(move |c| {
                let _ = send.send(work(c));
            }))
            .map_err(|_| "방문 기록이 응답하지 않습니다.")?;
        receive
            .await
            .map_err(|_| "방문 기록이 응답하지 않습니다.".to_string())?
    }
    pub fn visit(&self, url: &str, title: &str) {
        let Some(url) = clean_url(url) else { return };
        let title = title.to_owned();
        let at = now();
        let _ = self.sender.send(Box::new(move |c| {
            let _ = record(c, &url, &title, at);
        }));
    }
    pub fn title(&self, url: &str, title: &str) {
        let Some(url) = clean_url(url) else { return };
        let title = title.chars().take(160).collect::<String>();
        let _=self.sender.send(Box::new(move|c|{let _=c.execute("UPDATE visits SET title=?1 WHERE id=(SELECT id FROM visits WHERE url=?2 ORDER BY visited_at DESC,id DESC LIMIT 1)",params![title,url]);}));
    }
    pub async fn search(&self, kind: String, text: String, offset: usize) -> Result<Page, String> {
        self.run(move |c| query(c, &kind, &text, offset)).await
    }
    pub async fn change(
        &self,
        kind: String,
        action: String,
        id: String,
        url: String,
        title: String,
    ) -> Result<bool, String> {
        self.run(move |c| edit(c, &kind, &action, &id, &url, &title))
            .await
    }
    pub fn flush(&self) {
        let (s, r) = mpsc::channel();
        if self
            .sender
            .send(Box::new(move |_| {
                let _ = s.send(());
            }))
            .is_ok()
        {
            let _ = r.recv_timeout(Duration::from_secs(2));
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Preferences {
    pub save_passwords: bool,
    pub autofill: bool,
    pub restore_tabs: bool,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            save_passwords: true,
            autofill: true,
            restore_tabs: true,
        }
    }
}
impl Preferences {
    pub fn load(root: &Path) -> Self {
        match std::fs::read(root.join("preferences.json")) {
            Ok(b) => serde_json::from_slice(&b).unwrap_or(Self {
                save_passwords: false,
                autofill: false,
                ..Self::default()
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(_) => Self {
                save_passwords: false,
                autofill: false,
                ..Self::default()
            },
        }
    }
    pub fn save(&self, root: &Path) -> Result<(), String> {
        let p = root.join("preferences.pending");
        std::fs::write(
            &p,
            serde_json::to_vec(self).map_err(|_| "설정을 저장하지 못했습니다.")?,
        )
        .map_err(|_| "설정을 저장하지 못했습니다.")?;
        super::vpn::replace(&p, &root.join("preferences.json"))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn db() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        initialize(&c).unwrap();
        c
    }
    #[test]
    fn imports_only_same_profile_public_history_once_without_changing_source() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("web-profile/EBWebView/Default");
        std::fs::create_dir_all(&path).unwrap();
        let source = Connection::open(path.join("History")).unwrap();
        source.execute_batch("CREATE TABLE urls(id INTEGER,url TEXT,title TEXT,hidden INTEGER);CREATE TABLE visits(id INTEGER,url INTEGER,visit_time INTEGER);INSERT INTO urls VALUES(1,'https://example.com/callback?code=secret&item=cat','Old title',0),(2,'file:///C:/private','Private',0);INSERT INTO visits VALUES(1,1,13439223600000000),(2,2,13439223600000000);").unwrap();
        drop(source);
        let before = std::fs::read(path.join("History")).unwrap();
        let mut c = db();
        import_previous_history(&mut c, d.path()).unwrap();
        import_previous_history(&mut c, d.path()).unwrap();
        let page = query(&c, "history", "", 0).unwrap();
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].url, "https://example.com/callback?item=cat");
        assert_eq!(page.items[0].visited_at, 1794750000000);
        assert_eq!(before, std::fs::read(path.join("History")).unwrap());
    }
    #[test]
    fn search_paginates_newest_first_without_losing_equal_time_visits() {
        let c = db();
        for n in 0..113 {
            record(&c, &format!("https://example.com/{n}"), "앨범", n).unwrap();
        }
        let a = query(&c, "history", "", 0).unwrap();
        let b = query(&c, "history", "", a.next_offset).unwrap();
        let d = query(&c, "history", "", b.next_offset).unwrap();
        assert_eq!((a.items.len(), b.items.len(), d.items.len()), (50, 50, 13));
        assert_eq!(a.items[0].visited_at, 112);
        assert!(a.has_more && b.has_more && !d.has_more);
        assert_ne!(a.items[49].id, b.items[0].id);
    }
    #[test]
    fn literal_search_does_not_treat_wildcards_as_everything() {
        let c = db();
        record(&c, "https://example.com/a", "100%_match", 1).unwrap();
        record(&c, "https://example.com/b", "다른 제목", 2).unwrap();
        assert_eq!(query(&c, "history", "%_", 0).unwrap().items.len(), 1);
        assert!(query(&c, "invalid", "", 0).is_err());
    }
    #[test]
    fn removing_history_preserves_bookmarks_and_other_visits() {
        let c = db();
        record(&c, "https://example.com/a", "A", 1).unwrap();
        record(&c, "https://example.com/b", "B", 2).unwrap();
        edit(&c, "bookmarks", "add", "", "https://example.com/a", "A").unwrap();
        let row = &query(&c, "history", "", 0).unwrap().items[0];
        edit(&c, "history", "remove", &row.id, "", "").unwrap();
        assert_eq!(query(&c, "history", "", 0).unwrap().items.len(), 1);
        edit(&c, "history", "clear", "", "", "").unwrap();
        assert_eq!(query(&c, "bookmarks", "", 0).unwrap().items.len(), 1);
        assert!(edit(&c, "bookmarks", "status", "", "https://example.com/a", "").unwrap());
    }
    #[test]
    fn private_protocols_and_credential_fragments_do_not_enter_index() {
        for u in [
            "file:///C:/secret",
            "javascript:alert(1)",
            "http://127.0.0.1:42/",
            "about:blank",
            "https://user:password@example.com/",
        ] {
            assert!(clean_url(u).is_none());
        }
        assert_eq!(
            clean_url("https://example.com/callback?code=secret&title=cat#access_token=hidden")
                .unwrap(),
            "https://example.com/callback?title=cat"
        );
    }
    #[test]
    fn data_survives_reopen_and_duplicate_bookmark_does_not_multiply() {
        let d = tempfile::tempdir().unwrap();
        {
            let c = Connection::open(d.path().join("library.sqlite3")).unwrap();
            initialize(&c).unwrap();
            record(&c, "https://example.com/a", "Saved", 1).unwrap();
            for _ in 0..2 {
                edit(&c, "bookmarks", "add", "", "https://example.com/a", "Saved").unwrap();
            }
        }
        let c = Connection::open(d.path().join("library.sqlite3")).unwrap();
        initialize(&c).unwrap();
        assert_eq!(query(&c, "history", "", 0).unwrap().items.len(), 1);
        assert_eq!(query(&c, "bookmarks", "", 0).unwrap().items.len(), 1);
    }
    #[test]
    fn preference_roundtrip_and_corruption_never_enable_password_saving() {
        let d = tempfile::tempdir().unwrap();
        let p = Preferences {
            save_passwords: false,
            autofill: false,
            restore_tabs: false,
        };
        p.save(d.path()).unwrap();
        assert_eq!(Preferences::load(d.path()), p);
        std::fs::write(d.path().join("preferences.json"), b"broken").unwrap();
        assert!(!Preferences::load(d.path()).save_passwords);
    }
}
