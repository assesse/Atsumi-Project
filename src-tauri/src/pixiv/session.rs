//! Private, generation-fenced request authorization. Never serialize this module's leases.
use reqwest::{header::HeaderValue, Url};
use std::sync::{Arc, Mutex};

pub(super) const STALE: &str = "Pixiv 로그인 상태가 변경되었습니다. 다시 시도해 주세요.";
#[derive(Clone, Default)]
pub(super) struct Authority(Arc<Mutex<u64>>);
impl Authority {
    pub fn revision(&self) -> u64 {
        *self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
    pub fn invalidate(&self) {
        let mut value = self.0.lock().unwrap_or_else(|e| e.into_inner());
        *value = value.wrapping_add(1);
    }
    pub fn lease(&self, cookie: Option<HeaderValue>) -> Lease {
        Lease {
            authority: self.clone(),
            revision: self.revision(),
            cookie,
        }
    }
}

#[derive(Clone)]
pub(super) struct Lease {
    authority: Authority,
    revision: u64,
    cookie: Option<HeaderValue>,
}

// An in-memory feed scope keeps authorization identity, not its cookie. An old
// account's recommendation cursor must never authorize reads in a new session.
#[derive(Clone)]
pub(super) struct Scope {
    authority: Authority,
    revision: u64,
    authenticated: bool,
}
impl Scope {
    pub fn matches(&self, lease: &Lease) -> bool {
        Arc::ptr_eq(&self.authority.0, &lease.authority.0)
            && self.revision == lease.revision
            && self.authenticated == lease.authenticated()
    }
}
impl Default for Lease {
    fn default() -> Self {
        Authority::default().lease(None)
    }
}
impl Lease {
    pub fn scope(&self) -> Scope {
        Scope {
            authority: self.authority.clone(),
            revision: self.revision,
            authenticated: self.authenticated(),
        }
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn authenticated(&self) -> bool {
        self.cookie.is_some()
    }
    pub fn check(&self) -> Result<(), String> {
        self.with_current(|| Ok(()))
    }
    /// The publication check and rename share the same lock as logout invalidation.
    pub fn with_current<T>(&self, action: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
        let revision = self.authority.0.lock().unwrap_or_else(|e| e.into_inner());
        if *revision != self.revision {
            return Err(STALE.into());
        }
        action()
    }
    pub fn cookie_for(&self, url: &Url) -> Result<Option<HeaderValue>, String> {
        self.check()?;
        if url.scheme() != "https"
            || url.host_str() != Some("www.pixiv.net")
            || url.port().is_some_and(|p| p != 443)
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
        {
            return Ok(None);
        }
        Ok(self.cookie.clone())
    }
}

pub(super) fn session_header(value: &str) -> Option<HeaderValue> {
    if value.is_empty()
        || value.len() > 4096
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
    {
        return None;
    }
    let mut header = HeaderValue::from_str(&format!("PHPSESSID={value}")).ok()?;
    header.set_sensitive(true);
    Some(header)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn secret_header_never_reaches_another_origin_or_unsafe_value() {
        for value in ["", "a;b", "a\r\nb", "a=b", "a b"] {
            assert!(session_header(value).is_none());
        }
        assert!(session_header(&"x".repeat(4097)).is_none());
        let header = session_header("123_fixture").unwrap();
        assert!(header.is_sensitive());
        let lease = Authority::default().lease(Some(header));
        assert!(lease
            .cookie_for(&Url::parse("https://www.pixiv.net/ajax/user/extra").unwrap())
            .unwrap()
            .is_some());
        for url in [
            "https://i.pximg.net/img-original/test.jpg",
            "http://www.pixiv.net/",
            "https://www.pixiv.net.evil.test/",
            "https://www.pixiv.net:444/",
            "https://evil@www.pixiv.net/",
            "https://www.pixiv.net/#x",
        ] {
            assert!(lease
                .cookie_for(&Url::parse(url).unwrap())
                .unwrap()
                .is_none());
        }
    }
    #[test]
    fn logout_invalidates_all_old_work_but_not_other_authorities() {
        let authority = Authority::default();
        let other = Authority::default().lease(None);
        let lease = authority.lease(session_header("123_fixture"));
        assert!(lease.authenticated());
        authority.invalidate();
        assert!(lease.check().is_err());
        assert!(other.check().is_ok());
        let mut published = false;
        assert!(lease
            .with_current(|| {
                published = true;
                Ok(())
            })
            .is_err());
        assert!(!published);
        assert!(authority.lease(None).check().is_ok());
    }
}
