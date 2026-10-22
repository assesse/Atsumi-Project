use tauri::Url;
pub(super) fn valid_video(id: &str) -> bool {
    (1..=20).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_digit()) && !id.starts_with('0')
}
pub(super) fn player_url(id: &str) -> Result<Url, String> {
    if !valid_video(id) {
        return Err("SOOP 영상 주소를 확인해 주세요.".into());
    }
    Url::parse(&format!(
        "https://vod.sooplive.com/player/{id}/embed?showChat=true&replayType=1"
    ))
    .map_err(|_| "SOOP 영상 주소를 확인해 주세요.".into())
}
pub(super) fn matches_document(url: &Url, id: &str) -> bool {
    valid_video(id)
        && url.scheme() == "https"
        && url.host_str() == Some("vod.sooplive.com")
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
        && url.path() == format!("/player/{id}/embed")
        && url.query() == Some("showChat=true&replayType=1")
        && url.fragment().is_none()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn document_admission_is_exact_and_not_live_or_account_authority() {
        let id = "209244227";
        assert!(matches_document(&player_url(id).unwrap(), id));
        assert_eq!(player_url(id).unwrap().query(), Some("showChat=true&replayType=1"));
        for url in [
            "https://vod.sooplive.com/player/209244227/embed?showChat=true",
            "https://vod.sooplive.com/player/209244227/embed?showChat=true&replayType=0",
            "https://vod.sooplive.com/player/209244227/embed?showChat=true&replayType=1&replayType=0",
            "https://vod.sooplive.com/player/209244227/embed?replayType=1&showChat=true",
            "https://vod.sooplive.com/player/209244227/embed?showChat=true&replayType=1&fromApi=1",
            "https://vod.sooplive.com/player/209244228/embed?showChat=true&replayType=1",
            "http://vod.sooplive.com/player/209244227/embed?showChat=true",
            "https://vod.sooplive.com.evil.test/player/209244227/embed?showChat=true",
            "https://user@vod.sooplive.com/player/209244227/embed?showChat=true",
            "https://vod.sooplive.com:444/player/209244227/embed?showChat=true",
            "https://vod.sooplive.com/player/209244228/embed?showChat=true",
            "https://vod.sooplive.com/player/209244227/chat?showChat=true",
            "https://vod.sooplive.com/player/209244227/embed?showChat=true&fromApi=1",
            "https://vod.sooplive.com/player/209244227/embed?showChat=true#x",
            "https://login.sooplive.com/",
        ] {
            assert!(!matches_document(&url.parse().unwrap(), id), "{url}");
        }
        for id in [
            "",
            "0",
            "0209244227",
            "a",
            "1/2",
            "209244227?x=1",
            "123456789012345678901",
        ] {
            assert!(player_url(id).is_err());
        }
    }
}
