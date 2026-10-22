use super::PlatformMediaService;
use std::{
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom},
    path::PathBuf,
    sync::Mutex,
};
use tauri::http::{Request, Response, StatusCode};
static IO_SLOTS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(8);
pub(crate) fn try_io_permit(
) -> Result<tokio::sync::SemaphorePermit<'static>, tokio::sync::TryAcquireError> {
    IO_SLOTS.try_acquire()
}
fn allowed_origin<'a>(
    request: &'a Request<Vec<u8>>,
    development_url: Option<&reqwest::Url>,
) -> Result<Option<&'a str>, ()> {
    let Some(header) = request.headers().get("origin") else {
        return Ok(None);
    };
    let origin = header.to_str().map_err(|_| ())?;
    crate::frontend_origin::trusted_origin(origin, cfg!(debug_assertions), development_url)
        .then_some(Some(origin))
        .ok_or(())
}
pub(super) struct Session {
    pub library_id: String,
    file: Mutex<File>,
    length: u64,
}
impl Session {
    pub(super) fn new(path: PathBuf, library_id: String, length: u64) -> Result<Self, String> {
        super::storage::regular(&path)?;
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(1).custom_flags(0x00200000);
        }
        let file = options
            .open(path)
            .map_err(|_| "재생 파일을 열지 못했습니다.")?;
        if file
            .metadata()
            .map_err(|_| "재생 파일을 확인하지 못했습니다.")?
            .len()
            != length
        {
            return Err("파일이 변경됐습니다.".into());
        }
        Ok(Self {
            library_id,
            file: Mutex::new(file),
            length,
        })
    }
}
fn range(header: Option<&str>, length: u64) -> Result<(u64, u64), ()> {
    let Some(header) = header else {
        return Ok((0, length.min(1024 * 1024).saturating_sub(1)));
    };
    let raw = header.strip_prefix("bytes=").ok_or(())?;
    if raw.contains(',') {
        return Err(());
    }
    let (start, end) = raw.split_once('-').ok_or(())?;
    let (start, end) = if start.is_empty() {
        let suffix = end.parse::<u64>().map_err(|_| ())?;
        if suffix == 0 {
            return Err(());
        }
        (length.saturating_sub(suffix), length.saturating_sub(1))
    } else {
        (
            start.parse::<u64>().map_err(|_| ())?,
            if end.is_empty() {
                length.saturating_sub(1)
            } else {
                end.parse::<u64>().map_err(|_| ())?
            },
        )
    };
    if start >= length || end < start {
        return Err(());
    }
    Ok((
        start,
        end.min(length - 1)
            .min(start.saturating_add(1024 * 1024 - 1)),
    ))
}
pub fn respond(
    service: &PlatformMediaService,
    webview_label: &str,
    request: Request<Vec<u8>>,
) -> Response<Vec<u8>> {
    let denied = || {
        Response::builder()
            .status(StatusCode::FORBIDDEN)
            .body(Vec::new())
            .unwrap()
    };
    if webview_label != "main"
        || !matches!(request.method().as_str(), "GET" | "HEAD")
        || !request.body().is_empty()
        || request.uri().query().is_some()
    {
        return denied();
    }
    let Ok(origin) = allowed_origin(&request, service.inner.development_url.as_ref()) else {
        return denied();
    };
    if let Some(live) = request.uri().path().strip_prefix("/live/") {
        let Some((token, name)) = live.split_once('/') else {
            return denied();
        };
        if name.contains('/') || uuid::Uuid::parse_str(token).is_err() {
            return denied();
        }
        let Ok((bytes, mime)) = service.live_file(token, name) else {
            return Response::builder()
                .status(404)
                .header("Cache-Control", "no-store")
                .body(Vec::new())
                .unwrap();
        };
        let mut response = Response::builder()
            .status(200)
            .header("Content-Type", mime)
            .header("Content-Length", bytes.len().to_string())
            .header("Cache-Control", "no-store")
            .header("X-Content-Type-Options", "nosniff");
        if let Some(origin) = origin {
            response = response
                .header("Access-Control-Allow-Origin", origin)
                .header("Vary", "Origin");
        }
        return response
            .body(if request.method() == "HEAD" {
                Vec::new()
            } else {
                bytes
            })
            .unwrap_or_else(|_| denied());
    }
    let Some(token) = request
        .uri()
        .path()
        .strip_prefix('/')
        .and_then(|v| v.strip_suffix("/media.mp4"))
    else {
        return denied();
    };
    if uuid::Uuid::parse_str(token).is_err() {
        return denied();
    }
    let Ok(sessions) = service.inner.playback.lock() else {
        return denied();
    };
    let Some(session) = sessions.get(token) else {
        return denied();
    };
    let length = session.length;
    let parsed = range(
        request.headers().get("range").and_then(|v| v.to_str().ok()),
        length,
    );
    let Ok((start, end)) = parsed else {
        return Response::builder()
            .status(416)
            .header("Content-Range", format!("bytes */{length}"))
            .body(Vec::new())
            .unwrap();
    };
    let head = request.method() == "HEAD";
    let mut bytes = if head {
        Vec::new()
    } else {
        vec![0; (end - start + 1) as usize]
    };
    if !head {
        let Ok(mut file) = session.file.lock() else {
            return denied();
        };
        if file
            .seek(SeekFrom::Start(start))
            .and_then(|_| file.read_exact(&mut bytes))
            .is_err()
        {
            return denied();
        }
    }
    let mut response = Response::builder()
        .status(if head { 200 } else { 206 })
        .header("Content-Type", "video/mp4")
        .header("Accept-Ranges", "bytes")
        .header("Cache-Control", "no-store")
        .header("X-Content-Type-Options", "nosniff")
        .header("Content-Security-Policy", "sandbox")
        .header(
            "Content-Length",
            if head { length } else { end - start + 1 }.to_string(),
        );
    if !head {
        response = response.header("Content-Range", format!("bytes {start}-{end}/{length}"));
    }
    if let Some(origin) = origin {
        response = response
            .header("Access-Control-Allow-Origin", origin)
            .header("Vary", "Origin");
    }
    response.body(bytes).unwrap_or_else(|_| denied())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_single_ranges() {
        assert_eq!(range(Some("bytes=2-"), 10), Ok((2, 9)));
        assert_eq!(range(Some("bytes=-3"), 10), Ok((7, 9)));
        assert!(range(Some("bytes=1-2,4-5"), 10).is_err());
        assert_eq!(range(None, 10_000_000), Ok((0, 1048575)));
    }
}
#[cfg(test)]
mod admission_tests {
    use super::*;
    #[test]
    fn origin_is_exact_and_malformed_is_not_absent() {
        for value in [
            "null",
            "http://tauri.localhost/path",
            "http://tauri.localhost/",
            "https://example.com",
            "http://user@tauri.localhost",
        ] {
            let request = Request::builder()
                .header("Origin", value)
                .body(Vec::new())
                .unwrap();
            assert!(allowed_origin(&request, None).is_err());
        }
        let request = Request::builder()
            .header(
                "Origin",
                tauri::http::HeaderValue::from_bytes(&[0xff]).unwrap(),
            )
            .body(Vec::new())
            .unwrap();
        assert!(allowed_origin(&request, None).is_err());
        let request = Request::builder()
            .header("Origin", "http://tauri.localhost")
            .body(Vec::new())
            .unwrap();
        assert_eq!(
            allowed_origin(&request, None),
            Ok(Some("http://tauri.localhost"))
        );
        assert_eq!(allowed_origin(&Request::new(Vec::new()), None), Ok(None));
    }
    #[test]
    fn protocol_work_is_admitted_before_blocking() {
        let permits = (0..8).map(|_| try_io_permit().unwrap()).collect::<Vec<_>>();
        assert!(try_io_permit().is_err());
        drop(permits);
        assert!(try_io_permit().is_ok());
    }
}
