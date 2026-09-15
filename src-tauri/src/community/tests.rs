use super::*;
use std::cell::{Cell, RefCell};

struct MemoryStore {
    value: RefCell<String>,
    fail_write: Cell<bool>,
    fail_read: Cell<bool>,
}
impl MemoryStore {
    fn new() -> Self {
        Self {
            value: RefCell::new(serde_json::to_string(&Identity::Unissued).unwrap()),
            fail_write: Cell::new(false),
            fail_read: Cell::new(false),
        }
    }
}
impl IdentityStore for MemoryStore {
    fn load(&self) -> Result<Identity, String> {
        if self.fail_read.get() {
            return Err("unreadable".into());
        }
        serde_json::from_str(&self.value.borrow()).map_err(|_| "corrupt".into())
    }
    fn save(&self, identity: &Identity) -> Result<(), String> {
        if self.fail_write.get() {
            return Err("disk full".into());
        }
        *self.value.borrow_mut() = serde_json::to_string(identity).unwrap();
        Ok(())
    }
}
struct FakeTransport {
    routes: RefCell<Vec<(String, bool)>>,
    fail: Cell<bool>,
    reject: Cell<bool>,
    other_user: Cell<bool>,
}
impl FakeTransport {
    fn new() -> Self {
        Self {
            routes: RefCell::new(vec![]),
            fail: Cell::new(false),
            reject: Cell::new(false),
            other_user: Cell::new(false),
        }
    }
}
impl Transport for FakeTransport {
    fn post(&self, route: &str, _: Value, token: Option<&str>) -> Result<Value, RequestFailure> {
        self.routes
            .borrow_mut()
            .push((route.into(), token.is_some()));
        if self.fail.get() {
            return Err(RequestFailure {
                message: "test failure".into(),
                definitely_rejected: self.reject.get(),
            });
        }
        if route.starts_with("auth/") {
            return Ok(
                json!({"access_token":"secret-access","refresh_token":"secret-refresh","expires_in":3600,"user":{"id": if self.other_user.get() { "22222222-2222-4222-8222-222222222222" } else { "11111111-1111-4111-8111-111111111111" }}}),
            );
        }
        Ok(json!({"items": [], "nextCursor": null}))
    }
}

#[test]
fn community_public_reads_never_issue_identity_or_send_user_token() {
    let http = FakeTransport::new();
    read_request(
        &http,
        ReadRequest::Feed {
            source: None,
            cursor: None,
        },
    )
    .unwrap();
    read_request(
        &http,
        ReadRequest::Work {
            source: "hitomi".into(),
            work_id: "3657124".into(),
            cursor: None,
        },
    )
    .unwrap();
    assert_eq!(http.routes.borrow().len(), 3);
    assert!(http
        .routes
        .borrow()
        .iter()
        .all(|(route, token)| !*token && !route.starts_with("auth/")));
}
#[test]
fn community_issues_once_and_reuses_identity_after_reopening_store() {
    let store = MemoryStore::new();
    let http = FakeTransport::new();
    assert!(ensure_session(&store, &http, 100, false).is_err());
    let first = ensure_session(&store, &http, 100, true).unwrap();
    let reopened = MemoryStore {
        value: RefCell::new(store.value.borrow().clone()),
        fail_write: Cell::new(false),
        fail_read: Cell::new(false),
    };
    let next = ensure_session(&reopened, &http, 110, true).unwrap();
    assert_eq!(first.user_id, next.user_id);
    assert_eq!(http.routes.borrow().len(), 1);
}
#[test]
fn community_storage_failures_never_issue_or_replace_identity() {
    let store = MemoryStore::new();
    let http = FakeTransport::new();
    store.fail_write.set(true);
    assert!(ensure_session(&store, &http, 0, true).is_err());
    store.fail_write.set(false);
    store.fail_read.set(true);
    assert!(ensure_session(&store, &http, 0, true).is_err());
    assert!(http.routes.borrow().is_empty());
}
#[test]
fn community_unknown_signup_outcome_is_not_automatically_retried() {
    let store = MemoryStore::new();
    let http = FakeTransport::new();
    http.fail.set(true);
    assert!(ensure_session(&store, &http, 0, true).is_err());
    http.fail.set(false);
    assert!(ensure_session(&store, &http, 10, true).is_err());
    assert!(matches!(store.load().unwrap(), Identity::Issuing));
    assert_eq!(http.routes.borrow().len(), 1);
}
#[test]
fn community_explicit_signup_rejection_can_retry_without_pending_identity() {
    let store = MemoryStore::new();
    let http = FakeTransport::new();
    http.fail.set(true);
    http.reject.set(true);
    assert!(ensure_session(&store, &http, 0, true).is_err());
    assert!(matches!(store.load().unwrap(), Identity::Unissued));
}
#[test]
fn community_refresh_preserves_owner_and_does_not_signup_after_failure() {
    let store = MemoryStore::new();
    let http = FakeTransport::new();
    let original = ensure_session(&store, &http, 0, true).unwrap();
    let refreshed = ensure_session(&store, &http, 3600, false).unwrap();
    assert_eq!(refreshed.user_id, original.user_id);
    http.fail.set(true);
    assert!(ensure_session(&store, &http, 8000, true).is_err());
    http.fail.set(false);
    http.other_user.set(true);
    assert!(ensure_session(&store, &http, 8000, true).is_err());
    assert_eq!(
        http.routes
            .borrow()
            .iter()
            .filter(|(route, _)| route.ends_with("signup"))
            .count(),
        1
    );
    assert!(
        matches!(store.load().unwrap(), Identity::Ready { session } if session.user_id == original.user_id)
    );
}
#[test]
fn community_only_trusted_local_main_can_use_identity() {
    for url in [
        "https://chzzk.naver.com",
        "http://127.0.0.1:1421",
        "http://user@127.0.0.1:1420",
        "https://attacker.test",
    ] {
        assert!(!trusted_main("main", &url.parse().unwrap(), true));
    }
    assert!(!trusted_main(
        "official-browser",
        &"http://tauri.localhost".parse().unwrap(),
        false
    ));
    assert!(trusted_main(
        "main",
        &"http://tauri.localhost".parse().unwrap(),
        false
    ));
    assert!(trusted_main(
        "main",
        &"http://127.0.0.1:1420".parse().unwrap(),
        true
    ));
    assert!(!trusted_main(
        "main",
        &"http://127.0.0.1:1420".parse().unwrap(),
        false
    ));
}
#[test]
fn community_request_contract_rejects_invalid_work_before_network() {
    let http = FakeTransport::new();
    for id in ["", "0", "01", "-1", "../1", "123456789012345678901"] {
        assert!(validate_work("hitomi", id).is_err());
    }
    assert!(validate_work("chzzk", "1").is_err());
    let request: ReadRequest = serde_json::from_value(
        json!({"kind":"work","source":"hitomi","workId":"123","cursor":null}),
    )
    .unwrap();
    read_request(&http, request).unwrap();
    assert!(safe_server_error(500, &json!({"message":"secret-access"}))
        .find("secret-access")
        .is_none());
}

#[cfg(windows)]
#[test]
fn community_encrypted_identity_survives_factory_reset_and_detects_corruption() {
    let temp = tempfile::tempdir().unwrap();
    let data_dir = temp.path().join("roaming");
    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::write(data_dir.join("factory-reset.pending"), b"reset").unwrap();
    std::fs::write(data_dir.join("atsumi-next.sqlite3"), b"synthetic DB").unwrap();
    let path = temp
        .path()
        .join("local/community-identity/project.v1.dpapi");
    let store = vault::FileVault::open(path.clone()).unwrap();
    let http = FakeTransport::new();
    let original = ensure_session(&store, &http, 100, true).unwrap();
    assert!(!String::from_utf8_lossy(&std::fs::read(&path).unwrap()).contains("secret-refresh"));
    assert!(vault::FileVault::open(path.clone()).is_err()); // Cross-process exclusion.
    drop(store);
    crate::apply_pending_factory_reset(&data_dir).unwrap();
    let reopened = vault::FileVault::open(path.clone()).unwrap();
    assert_eq!(
        ensure_session(&reopened, &http, 120, true).unwrap().user_id,
        original.user_id
    );
    ensure_session(&reopened, &http, 4000, true).unwrap(); // Atomic overwrite/rotation.
    std::fs::write(&path, b"corrupt").unwrap();
    assert!(reopened.load().is_err());
    assert!(ensure_session(&reopened, &http, 4001, true).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"corrupt"); // Never silently erased.
}
