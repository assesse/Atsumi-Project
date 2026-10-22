//! SOOP-only document/lease policy. No CHZZK identifiers or account state.
use std::time::{Duration, Instant};
use tauri::Url;

pub(super) const PREFIX: &str = "ATSUMI_SOOP_V1:";
pub(super) const MAX_MESSAGE: usize = 190_000;
pub(super) fn valid_channel(value: &str) -> bool {
    (1..=40).contains(&value.len())
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
}
pub(super) fn valid_broadcast(value: &str) -> bool {
    !value.is_empty() && value.len() <= 20 && value.bytes().all(|b| b.is_ascii_digit())
}
fn https(url: &Url) -> bool {
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
}
pub(super) fn player_channel(url: &Url) -> Option<String> {
    if !https(url) || url.host_str() != Some("play.sooplive.com") {
        return None;
    }
    let parts = url.path().trim_matches('/').split('/').collect::<Vec<_>>();
    match parts.as_slice() {
        [id, "embed"] if valid_channel(id) => Some(id.to_ascii_lowercase()),
        [id, no, "embed"] if valid_channel(id) && valid_broadcast(no) => {
            Some(id.to_ascii_lowercase())
        }
        _ => None,
    }
}
pub(super) fn player_url(channel: &str, broadcast: &str) -> Result<Url, String> {
    if !valid_channel(channel) || !valid_broadcast(broadcast) {
        return Err("SOOP 방송 정보를 확인해 주세요.".into());
    }
    Url::parse(&format!(
        "https://play.sooplive.com/{channel}/{broadcast}/embed?fromApi=1"
    ))
    .map_err(|_| "SOOP 방송 주소를 만들지 못했습니다.".into())
}
pub(super) fn matches_player(url: &Url, channel: &str, broadcast: &str) -> bool {
    player_channel(url).as_deref() == Some(channel)
        && url.path().trim_matches('/') == format!("{channel}/{broadcast}/embed")
}
pub(super) fn account_url(url: &Url) -> bool {
    https(url)
        && matches!(
            url.host_str(),
            Some(
                "login.sooplive.com"
                    | "auth.sooplive.com"
                    | "member.sooplive.com"
                    | "www.sooplive.com"
            )
        )
}
pub(super) fn account_home(url: &Url) -> bool {
    https(url)
        && url.host_str() == Some("www.sooplive.com")
        && url.path() == "/"
        && url.query().is_none()
        && url.fragment().is_none()
}
pub(super) fn account_entry_url(logout: bool) -> Url {
    let value = if logout {
        "https://login.sooplive.com/app/LogOut.php?request_uri=https%3A%2F%2Fwww.sooplive.com%2F"
    } else {
        "https://login.sooplive.com/afreeca/login.php?szFrom=full&request_uri=https%3A%2F%2Fwww.sooplive.com%2F"
    };
    Url::parse(value).expect("fixed official account entry")
}
pub(super) fn account_observation_url(url: &Url) -> bool {
    account_home(url)
        || (account_url(url) && url.host_str() == Some("login.sooplive.com"))
}
pub(super) fn account_observation_matches(source: &str, current: &Url) -> bool {
    account_observation_url(current)
        && Url::parse(source).ok().as_ref() == Some(current)
}
pub(super) fn account_https_return(url: &Url) -> Option<Url> {
    if url.scheme() != "http"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.host_str() != Some("www.sooplive.com")
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return None;
    }
    Some(Url::parse("https://www.sooplive.com/").expect("fixed official home"))
}

/// Proof belongs to one explicit account-window intent, never to saved cookies.
#[derive(Default)]
pub(super) struct AccountFlow {
    nonce: Option<String>,
    epoch: u64,
    expected: bool,
    observed: Option<bool>,
    home_finished: bool,
    resume_pending: bool,
    observation_nonce: Option<String>,
    observation_revision: u64,
}
impl AccountFlow {
    pub fn nonce(&self) -> Option<String> {
        self.nonce.clone()
    }
    pub fn begin(&mut self, nonce: String, expected: bool) {
        self.nonce = Some(nonce);
        self.epoch = self.epoch.saturating_add(1);
        self.expected = expected;
        self.observed = None;
        self.observation_nonce = None;
        self.observation_revision = self.observation_revision.saturating_add(1);
        self.home_finished = false;
        self.resume_pending = false;
    }
    pub fn current(&self, nonce: &str) -> Option<u64> {
        (self.nonce.as_deref() == Some(nonce)).then_some(self.epoch)
    }
    pub fn reopen(&mut self, nonce: &str, expected: bool) -> Option<bool> {
        self.current(nonce)?;
        if self.expected == expected {
            return Some(false);
        }
        self.begin(nonce.to_owned(), expected);
        Some(true)
    }
    pub fn navigate(&mut self, nonce: &str) -> Option<u64> {
        self.current(nonce)?;
        self.epoch = self.epoch.saturating_add(1);
        self.home_finished = false;
        self.resume_pending = false;
        Some(self.epoch)
    }
    pub fn observe(&mut self, nonce: &str, epoch: u64, authenticated: bool) -> bool {
        if epoch == 0 || self.current(nonce) != Some(epoch) {
            return false;
        }
        // The official callback and the following home probe can confirm the
        // same result. Preserve close proof without refreshing receivers twice.
        if self.observed == Some(authenticated)
            && self.observation_nonce.as_deref() == Some(nonce)
        {
            return true;
        }
        self.observed = Some(authenticated);
        self.observation_nonce = Some(nonce.to_owned());
        self.observation_revision = self.observation_revision.saturating_add(1);
        true
    }
    pub fn observation_revision(&self) -> u64 {
        self.observation_revision
    }
    /// Caller holds the flow and current-document arm gates. Viewer lifetime is
    /// not account lifetime; only an accepted boolean may replace this evidence.
    pub fn retain_player(
        &self,
        document_revision: u64,
        refresh_pending: bool,
        incoming: Option<bool>,
        retained: &mut Option<bool>,
    ) -> bool {
        if refresh_pending || document_revision != self.observation_revision {
            return false;
        }
        let Some(value) = incoming else { return false };
        *retained = Some(value);
        true
    }
    pub fn accepts_refresh(&self, nonce: &str, revision: u64) -> bool {
        self.observation_nonce.as_deref() == Some(nonce) && self.observation_revision == revision
    }
    pub fn finish(&mut self, nonce: &str, url: &Url) -> Option<u64> {
        let epoch = self.current(nonce)?;
        self.home_finished = account_home(url);
        Some(epoch)
    }
    pub fn can_resume(&self, nonce: &str, epoch: u64) -> bool {
        self.current(nonce) == Some(epoch) && self.observed == Some(self.expected)
    }
    pub fn prepare_resume(&mut self, nonce: &str) -> Option<u64> {
        let epoch = self.current(nonce)?;
        if self.resume_pending || !self.can_resume(nonce, epoch) {
            return None;
        }
        self.resume_pending = true;
        Some(epoch)
    }
    pub fn can_close(&self, nonce: &str, epoch: u64) -> bool {
        self.can_resume(nonce, epoch) && self.home_finished
    }
    pub fn invalidate(&mut self, nonce: &str) {
        if self.current(nonce).is_some() {
            self.nonce = None;
            // A valid accepted account result survives normal window closure;
            // a newer explicit intent/observation, not navigation, invalidates refresh.
            self.home_finished = false;
            self.resume_pending = false;
        }
    }
}
pub(super) fn popup_url(url: &Url) -> bool {
    account_url(url)
        || https(url)
            && matches!(
                url.host_str(),
                Some(
                    "point.sooplive.com"
                        | "pay.sooplive.com"
                        | "adballoon.sooplive.com"
                        | "item.sooplive.com"
                        | "st.sooplive.com"
                )
            )
}

#[derive(Clone, Debug)]
pub(super) struct Arm {
    pub id: String,
    pub epoch: u64,
    pub created: Instant,
}
impl Arm {
    pub fn accepts(&self, id: &str, epoch: u64, now: Instant) -> bool {
        self.id == id
            && self.epoch == epoch
            && now.saturating_duration_since(self.created) <= Duration::from_secs(25)
    }
}
pub(super) fn validate_envelope(
    source: &str,
    label: &str,
    expected_label: &str,
    channel: &str,
    epoch: u64,
    current_epoch: u64,
    body: &str,
) -> bool {
    label == expected_label
        && epoch > 0
        && epoch == current_epoch
        && body.len() <= MAX_MESSAGE
        && Url::parse(source)
            .ok()
            .as_ref()
            .and_then(player_channel)
            .as_deref()
            == Some(channel)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn account_home_observation_and_official_logout_entry_are_exact() {
        let home: Url = "https://www.sooplive.com/".parse().unwrap();
        assert!(account_observation_matches(home.as_str(), &home));
        let logout = account_entry_url(true);
        assert_eq!(logout.path(), "/app/LogOut.php");
        assert_eq!(logout.query_pairs().collect::<Vec<_>>(), vec![("request_uri".into(), home.as_str().into())]);
        assert!(account_observation_url(&logout));
        assert_eq!(account_entry_url(false).path(), "/afreeca/login.php");
        for source in ["http://www.sooplive.com/", "https://www.sooplive.com/?x=1", "https://www.sooplive.com/#x", "https://www.sooplive.com/other", "https://member.sooplive.com/app/campaign_pw.php", "https://www.sooplive.com.evil.test/", "https://user@www.sooplive.com/", "https://www.sooplive.com:444/"] {
            let url: Url = source.parse().unwrap();
            assert!(!account_observation_url(&url), "{source}");
            assert!(!account_observation_matches(source, &home), "{source}");
        }
        assert!(!account_observation_matches(logout.as_str(), &home));
    }
    #[test]
    fn late_home_proof_completes_current_intent_without_reloading_twice() {
        let home: Url = "https://www.sooplive.com/".parse().unwrap();
        let mut flow = AccountFlow::default();
        flow.begin("retained".into(), true);
        let epoch = flow.navigate("retained").unwrap();
        flow.finish("retained", &home);
        assert!(!flow.can_close("retained", epoch));
        assert!(flow.observe("retained", epoch, true));
        assert!(flow.can_close("retained", epoch));
        let revision = flow.observation_revision();
        assert!(flow.observe("retained", epoch, true));
        assert_eq!(flow.observation_revision(), revision);
        let next = flow.navigate("retained").unwrap();
        flow.finish("retained", &home);
        assert!(!flow.observe("retained", epoch, false));
        assert!(!flow.observe("old-window", next, false));
        assert!(flow.observe("retained", next, true));
        assert_eq!(flow.observation_revision(), revision);
        assert!(flow.can_close("retained", next));
        flow.reopen("retained", false);
        let logout = flow.current("retained").unwrap();
        flow.finish("retained", &home);
        assert!(!flow.can_close("retained", logout));
        assert!(flow.observe("retained", logout, false));
        assert!(flow.can_close("retained", logout));
    }
    #[test]
    fn player_account_retains_only_current_non_null_boolean() {
        let flow = AccountFlow::default();
        let mut retained = None;
        assert!(!flow.retain_player(0, false, None, &mut retained));
        assert_eq!(retained, None);
        assert!(flow.retain_player(0, false, Some(true), &mut retained));
        assert!(!flow.retain_player(0, false, None, &mut retained));
        assert_eq!(retained, Some(true));
        assert!(flow.retain_player(0, false, Some(false), &mut retained));
        assert_eq!(retained, Some(false));
        assert!(!flow.retain_player(0, true, Some(true), &mut retained));
        assert_eq!(retained, Some(false));
        assert_eq!(flow.observation_revision(), 0, "player polling cannot retag documents");
    }
    #[test]
    fn logout_callback_wins_before_or_during_deferred_player_refresh() {
        let mut flow = AccountFlow::default();
        let old_document = flow.observation_revision();
        let mut retained = None;
        assert!(flow.retain_player(old_document, false, Some(true), &mut retained));
        flow.begin("logout".into(), false);
        let during_intent = flow.observation_revision();
        assert!(!flow.retain_player(old_document, false, Some(false), &mut retained));
        assert_eq!(retained, Some(true), "an intent is not a confirmed logout");
        let epoch = flow.current("logout").unwrap();
        assert!(flow.observe("logout", epoch, false));
        retained = Some(false); // Same validated callback commit as the native host.
        for revision in [old_document, during_intent] {
            for pending in [false, true] {
                assert!(!flow.retain_player(revision, pending, Some(true), &mut retained));
                assert_eq!(retained, Some(false));
            }
        }
        let reloaded = flow.observation_revision();
        assert!(!flow.retain_player(reloaded, true, Some(true), &mut retained));
        assert!(flow.retain_player(reloaded, false, Some(false), &mut retained));
    }
    #[test]
    fn newer_login_and_normal_close_do_not_restore_old_player_evidence() {
        let mut flow = AccountFlow::default();
        flow.begin("one".into(), false);
        let old_document = flow.observation_revision();
        let old_epoch = flow.current("one").unwrap();
        assert!(flow.observe("one", old_epoch, false));
        let mut retained = Some(false);
        assert_eq!(retained, Some(false));
        assert_eq!(flow.reopen("one", true), Some(true));
        let current = flow.current("one").unwrap();
        assert!(flow.observe("one", current, true));
        retained = Some(true);
        assert!(!flow.retain_player(old_document, false, Some(false), &mut retained));
        assert_eq!(retained, Some(true));
        let revision = flow.observation_revision();
        flow.invalidate("one");
        assert_eq!(flow.observation_revision(), revision);
        assert_eq!(retained, Some(true), "closing a window is not logout");
        assert!(flow.retain_player(revision, false, Some(false), &mut retained));
        assert_eq!(retained, Some(false), "a fresh current document can update the observation");
    }
    #[test]
    fn exact_soop_realm_only() {
        let good = player_url("phonics1", "297833919").unwrap();
        assert_eq!(player_channel(&good).as_deref(), Some("phonics1"));
        for bad in [
            "http://play.sooplive.com/phonics1/embed",
            "https://play.sooplive.com.evil.test/phonics1/embed",
            "https://evil@play.sooplive.com/phonics1/embed",
            "https://play.sooplive.com:444/phonics1/embed",
            "https://chzzk.naver.com/phonics1/embed",
            "https://play.sooplive.com/phonics1/other",
            "https://play.sooplive.com/a%2Fb/embed",
        ] {
            assert!(player_channel(&Url::parse(bad).unwrap()).is_none(), "{bad}");
        }
    }
    #[test]
    fn only_native_current_epoch_and_label_accept() {
        let source = "https://play.sooplive.com/phonics1/297833919/embed?fromApi=1";
        assert!(validate_envelope(
            source, "soop-1", "soop-1", "phonics1", 1, 1, "{}"
        ));
        assert!(!validate_envelope(
            source, "chzzk-1", "soop-1", "phonics1", 1, 1, "{}"
        ));
        assert!(!validate_envelope(
            source, "soop-1", "soop-1", "phonics1", 1, 2, "{}"
        ));
        assert!(!validate_envelope(
            source, "soop-1", "soop-1", "other", 1, 1, "{}"
        ));
        assert!(!validate_envelope(
            source,
            "soop-1",
            "soop-1",
            "phonics1",
            1,
            1,
            &"x".repeat(MAX_MESSAGE + 1)
        ));
    }
    #[test]
    fn arm_is_bounded_to_epoch_and_time() {
        let now = Instant::now();
        let arm = Arm {
            id: uuid::Uuid::new_v4().to_string(),
            epoch: 3,
            created: now,
        };
        assert!(arm.accepts(&arm.id, 3, now + Duration::from_secs(24)));
        assert!(!arm.accepts(&arm.id, 4, now));
        assert!(!arm.accepts("other", 3, now));
        assert!(!arm.accepts(&arm.id, 3, now + Duration::from_secs(26)));
    }
    #[test]
    fn account_popup_does_not_widen_player_or_cross_provider() {
        assert!(account_url(
            &Url::parse("https://login.sooplive.com/afreeca/login.php").unwrap()
        ));
        for bad in [
            "https://nid.naver.com/nidlogin.login",
            "https://accounts.google.com/",
            "https://login.sooplive.com.evil.test/",
            "https://play.sooplive.com/phonics1/embed",
            "file:///C:/secret",
        ] {
            assert!(!account_url(&Url::parse(bad).unwrap()));
        }
    }
    #[test]
    fn same_channel_different_broadcast_cannot_use_receiver() {
        let good = player_url("fixture", "1234").unwrap();
        assert!(matches_player(&good, "fixture", "1234"));
        assert!(!matches_player(&good, "fixture", "5678"));
        assert!(!matches_player(
            &"https://play.sooplive.com/fixture/embed".parse().unwrap(),
            "fixture",
            "1234"
        ));
    }
    #[test]
    fn account_completion_requires_callback_and_home_after_campaign() {
        let home = Url::parse("https://www.sooplive.com/").unwrap();
        let campaign = Url::parse("https://member.sooplive.com/app/campaign_pw.php").unwrap();
        let mut flow = AccountFlow::default();
        flow.begin("one".into(), true);
        let epoch = flow.navigate("one").unwrap();
        flow.finish("one", &home);
        assert!(
            !flow.can_close("one", epoch),
            "home alone is not account proof"
        );
        assert!(flow.observe("one", epoch, true));
        let campaign_epoch = flow.navigate("one").unwrap();
        flow.finish("one", &campaign);
        assert!(!flow.can_close("one", campaign_epoch));
        assert!(flow.can_resume("one", campaign_epoch));
        assert_eq!(flow.prepare_resume("one"), Some(campaign_epoch));
        assert_eq!(
            flow.prepare_resume("one"),
            None,
            "one queued home continuation only"
        );
        let done = flow.navigate("one").unwrap();
        flow.finish("one", &home);
        assert!(flow.can_close("one", done));
    }
    #[test]
    fn stale_completion_and_https_resume_cannot_change_new_window_or_navigation() {
        let mut flow = AccountFlow::default();
        flow.begin("old".into(), true);
        let old = flow.current("old").unwrap();
        flow.observe("old", old, true);
        flow.finish("old", &Url::parse("https://www.sooplive.com/").unwrap());
        flow.navigate("old");
        assert!(!flow.can_close("old", old));
        assert!(!flow.can_resume("old", old));
        flow.begin("new".into(), true);
        assert!(!flow.observe("old", old, true));
        assert!(!flow.can_resume("old", old));
        flow.invalidate("old");
        let next = flow.current("new").unwrap();
        flow.finish("new", &Url::parse("https://www.sooplive.com/").unwrap());
        assert!(!flow.can_close("new", next));
        flow.observe("new", next, true);
        flow.begin("new".into(), false);
        assert!(
            !flow.can_close("new", next),
            "new logout intent cannot reuse login proof"
        );
        let logout = flow.current("new").unwrap();
        assert!(flow.observe("new", logout, false));
        flow.finish("new", &Url::parse("https://www.sooplive.com/").unwrap());
        assert!(flow.can_close("new", logout));
    }
    #[test]
    fn legacy_account_home_upgrade_is_exact_and_does_not_widen_account_policy() {
        let legacy = Url::parse("http://www.sooplive.com").unwrap();
        assert!(!account_url(&legacy));
        assert_eq!(
            account_https_return(&legacy).unwrap().as_str(),
            "https://www.sooplive.com/"
        );
        for value in [
            "http://www.sooplive.com/?request_uri=x",
            "http://www.sooplive.com/#x",
            "http://www.sooplive.com/login",
            "http://login.sooplive.com/",
            "http://www.sooplive.com.evil.test/",
            "http://user@www.sooplive.com/",
            "http://www.sooplive.com:8080/",
            "https://www.sooplive.com/",
        ] {
            assert!(
                account_https_return(&Url::parse(value).unwrap()).is_none(),
                "{value}"
            );
        }
    }
    #[test]
    fn second_login_success_completes_but_initial_incomplete_login_does_not() {
        let mut flow = AccountFlow::default();
        flow.begin("login".into(), true);
        let initial = flow.current("login").unwrap();
        assert!(flow.observe("login", initial, false));
        assert!(flow.prepare_resume("login").is_none());
        let second = flow.navigate("login").unwrap();
        assert!(flow.observe("login", second, true));
        let home = flow.navigate("login").unwrap();
        flow.finish("login", &Url::parse("https://www.sooplive.com/").unwrap());
        assert!(flow.can_close("login", home));
    }
    #[test]
    fn accepted_account_revision_survives_home_but_not_a_new_intent_or_observation() {
        let mut flow = AccountFlow::default();
        flow.begin("login".into(), true);
        let initial = flow.current("login").unwrap();
        flow.observe("login", initial, true);
        let old = flow.observation_revision();
        flow.navigate("login");
        assert!(
            flow.accepts_refresh("login", old),
            "normal return must not lose the accepted result"
        );
        flow.invalidate("login");
        assert!(
            flow.accepts_refresh("login", old),
            "normal close must not lose deferred receiver refresh"
        );
        flow.begin("logout".into(), false);
        assert!(!flow.accepts_refresh("login", old));
        let next = flow.current("logout").unwrap();
        flow.observe("logout", next, false);
        let latest = flow.observation_revision();
        assert!(!flow.accepts_refresh("login", old));
        assert!(flow.accepts_refresh("logout", latest));
        flow.observe("logout", next, true);
        assert!(
            !flow.accepts_refresh("logout", latest),
            "late worker cannot refresh an older observation"
        );
    }
    #[test]
    fn reopen_preserves_same_intent_security_page_but_replaces_opposite_intent() {
        let mut flow = AccountFlow::default();
        flow.begin("account".into(), false);
        let original = flow.current("account").unwrap();
        flow.observe("account", original, false);
        let old = flow.observation_revision();
        assert_eq!(flow.reopen("account", false), Some(false));
        assert_eq!(flow.current("account"), Some(original));
        assert_eq!(flow.reopen("account", true), Some(true));
        assert!(!flow.accepts_refresh("account", old));
        assert!(!flow.can_close("account", original));
        let login = flow.current("account").unwrap();
        assert!(flow.observe("account", login, true));
        flow.finish("account", &Url::parse("https://www.sooplive.com/").unwrap());
        assert!(flow.can_close("account", login));
    }
}
