//! Local-only media transport. A token resolves to one already-open completed
//! media handle; the URL grammar cannot name paths, hosts, or source filenames.
use super::*;
use std::{
    io::{Read, Seek, SeekFrom},
    sync::atomic::{AtomicUsize, Ordering},
};
use tauri::http::{header, Method, Request, Response, StatusCode};

pub(super) const MAX_RANGE_BYTES: u64 = 1024 * 1024;
static ACTIVE_REQUESTS: AtomicUsize = AtomicUsize::new(0);
struct RequestLease;
impl RequestLease {
    fn acquire() -> Option<Self> {
        ACTIVE_REQUESTS
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                (count < 32).then_some(count + 1)
            })
            .ok()
            .map(|_| Self)
    }
}
impl Drop for RequestLease {
    fn drop(&mut self) {
        ACTIVE_REQUESTS.fetch_sub(1, Ordering::AcqRel);
    }
}

impl ReplayService {
    /// Call on a blocking worker. The caller must pass the actual protocol
    /// context Webview label, never a request-supplied header or query value.
    pub fn media_response(
        &self,
        request: &Request<Vec<u8>>,
        webview_label: &str,
    ) -> Response<Vec<u8>> {
        let origin = request
            .headers()
            .get(header::ORIGIN)
            .and_then(|value| value.to_str().ok());
        if webview_label != "main" || origin.is_some_and(|origin| !trusted_origin(origin)) {
            return response(StatusCode::FORBIDDEN, vec![], None, None, None);
        }
        let Some(_lease) = RequestLease::acquire() else {
            return response(StatusCode::TOO_MANY_REQUESTS, vec![], None, None, origin);
        };
        if request.method() != Method::GET && request.method() != Method::HEAD {
            let mut result = response(StatusCode::METHOD_NOT_ALLOWED, vec![], None, None, origin);
            result
                .headers_mut()
                .insert(header::ALLOW, "GET, HEAD".parse().unwrap());
            return result;
        }
        if request.uri().query().is_some() || !request.body().is_empty() {
            return response(StatusCode::BAD_REQUEST, vec![], None, None, origin);
        }
        let path = request.uri().path();
        let Some(path) = path.strip_prefix('/') else {
            return response(StatusCode::BAD_REQUEST, vec![], None, None, origin);
        };
        let parts = path.split('/').collect::<Vec<_>>();
        if !matches!(parts.len(), 1 | 3) || !valid_token(parts[0]) {
            return response(StatusCode::BAD_REQUEST, vec![], None, None, origin);
        }
        let session = match self.session(parts[0]) {
            Ok(session) => session,
            Err(_) => return response(StatusCode::NOT_FOUND, vec![], None, None, origin),
        };
        if parts.len() == 3 && parts[1] == "asset" {
            if parts[1] != "asset"
                || parts[2].len() != 64
                || !parts[2]
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            {
                return response(StatusCode::BAD_REQUEST, vec![], None, None, origin);
            }
            if !index::asset_allowed(&session, parts[2]) {
                return response(StatusCode::NOT_FOUND, vec![], None, None, origin);
            }
            let asset =
                super::super::replay_assets::read_recording(&session.recording_root, parts[2])
                    .or_else(|_| {
                        super::super::replay_assets::read_cached(&self.inner.data_dir, parts[2])
                    });
            return match asset {
                Ok(asset) if session.valid().is_ok() => {
                    let length = asset.bytes.len() as u64;
                    response(
                        StatusCode::OK,
                        if request.method() == Method::HEAD {
                            vec![]
                        } else {
                            asset.bytes
                        },
                        Some(&asset.mime),
                        Some(length),
                        origin,
                    )
                }
                _ => response(StatusCode::NOT_FOUND, vec![], None, None, origin),
            };
        }
        let (media_file, stamp) = if parts.len() == 3 {
            let Some(index) = (parts[1] == "part")
                .then(|| decimal(parts[2]))
                .flatten()
                .and_then(|n| usize::try_from(n).ok())
            else {
                return response(StatusCode::BAD_REQUEST, vec![], None, None, origin);
            };
            let Some(part) = session.media_parts.get(index) else {
                return response(StatusCode::NOT_FOUND, vec![], None, None, origin);
            };
            (&part.file, &part.stamp)
        } else {
            (&session.media, &session.media_stamp)
        };
        let mut media = match media_file.lock() {
            Ok(media) => media,
            Err(_) => return response(StatusCode::NOT_FOUND, vec![], None, None, origin),
        };
        if session.cancel.load(Ordering::Acquire) || !stamp.matches(&media) {
            return response(StatusCode::NOT_FOUND, vec![], None, None, origin);
        }
        let total = stamp.len;
        if request.method() == Method::HEAD {
            return response(
                StatusCode::OK,
                vec![],
                Some(&session.mime_type),
                Some(total),
                origin,
            );
        }
        if request.headers().contains_key(header::IF_RANGE) {
            return response(
                StatusCode::BAD_REQUEST,
                b"If-Range is not supported for immutable replay sessions".to_vec(),
                None,
                None,
                origin,
            );
        }
        let ranges = request
            .headers()
            .get_all(header::RANGE)
            .iter()
            .collect::<Vec<_>>();
        let range = match ranges.as_slice() {
            [] if total <= MAX_RANGE_BYTES => None,
            [] => {
                return response(
                    StatusCode::BAD_REQUEST,
                    b"Range required".to_vec(),
                    None,
                    None,
                    origin,
                )
            }
            [value] => match value
                .to_str()
                .ok()
                .and_then(|value| byte_range(value, total))
            {
                Some(range) => Some(range),
                None => return unsatisfied(total, origin),
            },
            _ => return unsatisfied(total, origin),
        };
        let (start, end) = range.unwrap_or((0, total.saturating_sub(1)));
        let length = if total == 0 { 0 } else { end - start + 1 };
        if length > MAX_RANGE_BYTES {
            return response(StatusCode::BAD_REQUEST, vec![], None, None, origin);
        }
        let mut bytes = vec![0; length as usize];
        if media
            .seek(SeekFrom::Start(start))
            .and_then(|_| media.read_exact(&mut bytes))
            .is_err()
            || !stamp.matches(&media)
            || session.cancel.load(Ordering::Acquire)
        {
            return response(StatusCode::NOT_FOUND, vec![], None, None, origin);
        }
        let mut result = response(
            if range.is_some() {
                StatusCode::PARTIAL_CONTENT
            } else {
                StatusCode::OK
            },
            bytes,
            Some(&session.mime_type),
            Some(length),
            origin,
        );
        if range.is_some() {
            if let Ok(value) = format!("bytes {start}-{end}/{total}").parse() {
                result.headers_mut().insert(header::CONTENT_RANGE, value);
            }
        }
        result
    }
}

fn trusted_origin(origin: &str) -> bool {
    matches!(
        origin,
        "tauri://localhost" | "http://tauri.localhost" | "https://tauri.localhost"
    ) || cfg!(debug_assertions) && origin == "http://127.0.0.1:1420"
}
fn decimal(value: &str) -> Option<u64> {
    if value.is_empty() || value.len() > 20 || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    value.parse().ok()
}
fn byte_range(value: &str, total: u64) -> Option<(u64, u64)> {
    if total == 0 || value.len() > 64 {
        return None;
    }
    let value = value.strip_prefix("bytes=")?;
    let (first, last) = value.split_once('-')?;
    if first.is_empty() {
        let count = decimal(last)?;
        if count == 0 {
            return None;
        }
        let start = total.saturating_sub(count);
        return Some((
            start,
            start.saturating_add(MAX_RANGE_BYTES - 1).min(total - 1),
        ));
    }
    let start = decimal(first)?;
    if start >= total {
        return None;
    }
    let end = if last.is_empty() {
        total - 1
    } else {
        decimal(last)?.min(total - 1)
    };
    if end < start {
        return None;
    }
    Some((start, end.min(start.saturating_add(MAX_RANGE_BYTES - 1))))
}
fn response(
    status: StatusCode,
    bytes: Vec<u8>,
    mime: Option<&str>,
    length: Option<u64>,
    origin: Option<&str>,
) -> Response<Vec<u8>> {
    let mut builder = Response::builder()
        .status(status)
        .header(header::CACHE_CONTROL, "no-store")
        .header(header::ACCEPT_RANGES, "bytes")
        .header("X-Content-Type-Options", "nosniff")
        .header("Content-Security-Policy", "default-src 'none'; sandbox")
        .header(
            header::CONTENT_LENGTH,
            length.unwrap_or(bytes.len() as u64).to_string(),
        );
    if let Some(mime) = mime {
        builder = builder.header(header::CONTENT_TYPE, mime);
    }
    if let Some(origin) = origin {
        builder = builder
            .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin)
            .header(header::VARY, "Origin");
    }
    builder
        .body(bytes)
        .unwrap_or_else(|_| Response::new(vec![]))
}
fn unsatisfied(total: u64, origin: Option<&str>) -> Response<Vec<u8>> {
    let mut result = response(
        StatusCode::RANGE_NOT_SATISFIABLE,
        vec![],
        None,
        None,
        origin,
    );
    if let Ok(value) = format!("bytes */{total}").parse() {
        result.headers_mut().insert(header::CONTENT_RANGE, value);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn byte_ranges_are_single_bounded_and_seekable() {
        assert_eq!(byte_range("bytes=10-19", 100), Some((10, 19)));
        assert_eq!(byte_range("bytes=90-", 100), Some((90, 99)));
        assert_eq!(byte_range("bytes=-10", 100), Some((90, 99)));
        assert_eq!(
            byte_range("bytes=0-", 100_000_000),
            Some((0, MAX_RANGE_BYTES - 1))
        );
        for invalid in [
            "bytes=100-",
            "bytes=2-1",
            "bytes=0-2,4-6",
            "bytes=+1-2",
            "bytes=-0",
            "bytes=0-18446744073709551616",
            "bytes= 0-1",
            "Bytes=0-1",
            "bytes=1-2-3",
        ] {
            assert!(byte_range(invalid, 100).is_none(), "{invalid}");
        }
        assert!(byte_range("bytes=0-", 0).is_none());
    }
    #[test]
    fn remote_origins_are_never_trusted() {
        assert!(trusted_origin("http://tauri.localhost"));
        for origin in [
            "https://chzzk.naver.com",
            "https://tauri.localhost.evil.example",
            "null",
            "file://",
            "http://127.0.0.1:1421",
        ] {
            assert!(!trusted_origin(origin));
        }
    }
}
