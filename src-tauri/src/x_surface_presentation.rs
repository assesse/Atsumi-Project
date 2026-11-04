//! Presentation of the current preview document, never the enclosing quote/reply.
use super::*;
#[derive(Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Presentation {
    pub path: String,
    pub name: String,
    pub first_line: String,
    pub avatar: Option<String>,
    #[serde(skip)]
    pub avatar_data: Option<String>,
}
impl Presentation {
    fn parse(value: &str) -> Option<Self> {
        let mut result: Self = serde_json::from_str(value).ok()?;
        if !navigation_path(&result.path) || result.path.len() > 256
            || !clean(&result.name, 120) || result.name.trim().is_empty()
            || !clean(&result.first_line, 200) { return None; }
        if let Some(url) = &result.avatar {
            // Only the exact observed public portrait. No rendition rewriting.
            if super::super::avatar_url(url).is_err() { return None; }
        }
        result.name = result.name.trim().into();
        result.first_line = result.first_line.trim().into();
        Some(result)
    }
}
fn clean(text: &str, limit: usize) -> bool {
    text.chars().count() <= limit && !text.chars().any(char::is_control)
}
pub(super) fn observe(session: &XSession, app: &AppHandle, pane: &Arc<Pane>, value: &str) {
    if pane.feed != "preview" || !interaction_allowed(session, pane) { return; }
    let Some(mut observed) = Presentation::parse(value) else {
        #[cfg(debug_assertions)] super::super::debug_shape("official-surface", "presentation", &serde_json::json!({"phase":"invalid"}));
        return;
    };
    let Some(current) = app.get_webview(&pane.label).and_then(|view| view.url().ok()) else { return; };
    if !allowed(&current) || current.path() != observed.path { return; }
    let fetch = {
        let mut data = session.surfaces.lock().unwrap_or_else(|p| p.into_inner());
        let Some(tab) = data.preview_tabs.iter_mut().find(|tab| tab.pane.label == pane.label && tab.path == observed.path) else { return; };
        let same_avatar = tab.presentation.as_ref().is_some_and(|old| old.path == observed.path && old.avatar == observed.avatar);
        if let Some(old) = tab.presentation.as_ref().filter(|_|same_avatar) {
            observed.avatar_data = old.avatar_data.clone();
            if old.name == observed.name && old.first_line == observed.first_line { return; }
        }
        let fetch = (!same_avatar).then(||observed.avatar.clone()).flatten();
        tab.presentation = Some(observed.clone());
        fetch
    };
    #[cfg(debug_assertions)] super::super::debug_shape("official-surface", "presentation", &serde_json::json!({"phase":"accepted"}));
    changed(session, app);
    let Some(source) = fetch else { return; };
    let host = session.clone(); let app = app.clone(); let pane = pane.clone();
    tauri::async_runtime::spawn(async move {
        let Ok(_permit) = host.image_slots.acquire().await else { return; };
        if !is_current(&host, &pane) { return; }
        let url = source.clone();
        let result = tauri::async_runtime::spawn_blocking(move ||
            super::super::account_presentation::fetch_avatar(&url)).await;
        let Ok(Ok(image)) = result else { return; };
        if !is_current(&host, &pane) { return; }
        let mut data = host.surfaces.lock().unwrap_or_else(|p| p.into_inner());
        let Some(tab) = data.preview_tabs.iter_mut().find(|tab|tab.pane.label == pane.label) else { return; };
        let Some(presentation) = tab.presentation.as_mut().filter(|p|p.path == observed.path && p.avatar.as_deref() == Some(&source)) else { return; };
        if tab.path != observed.path { return; }
        presentation.avatar_data = Some(image);
        drop(data);
        changed(&host, &app);
    });
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_plain_presentation_and_exact_public_portrait() {
        let good = r#"{"path":"/author/status/123456","name":"작성자","firstLine":"첫 줄","avatar":"https://pbs.twimg.com/profile_images/123/photo_normal.jpg"}"#;
        assert!(Presentation::parse(good).is_some());
        assert!(Presentation::parse(&good.replace("pbs.twimg.com", "example.com")).is_none());
        assert!(Presentation::parse(&good.replace("첫 줄", "first\\nsecond")).is_none());
        assert!(Presentation::parse(&good.replace("/author/status/123456", "/settings")).is_none());
        assert!(Presentation::parse(&good.replace("첫 줄", &"a".repeat(201))).is_none());
    }
}
