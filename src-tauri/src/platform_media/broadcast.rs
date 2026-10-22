//! Receiver-bound identity, not a channel's most recently observed broadcast.
use super::{MediaJob, MediaRef};
use serde::{Deserialize, Deserializer};

pub(super) const IDENTITY_ERROR: &str =
    "방송이 변경되었거나 방송 식별 정보를 확인하지 못했습니다. 받은 파일은 보존합니다.";

#[derive(Clone, Debug)]
pub(super) struct Binding {
    id: String,
    uploader: Option<String>,
}
fn video_id(id: &str) -> bool {
    id.len() == 11
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
}
fn channel_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 40
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
}
fn soop_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 20
        && id.bytes().all(|b| b.is_ascii_digit())
        && id
            .parse::<u64>()
            .is_ok_and(|n| n > 0 && n.to_string() == id)
}
pub(super) fn valid(reference: &MediaRef, id: &str) -> bool {
    reference.kind == "live"
        && match reference.provider.as_str() {
            "youtube" => video_id(id) && id == reference.external_id,
            "soop" => channel_id(&reference.external_id) && soop_id(id),
            _ => false,
        }
}
impl Binding {
    pub(super) fn preflight(
        reference: &MediaRef,
        metadata: &serde_json::Value,
        station_id: Option<&str>,
        scheduled_id: Option<&str>,
    ) -> Result<Self, String> {
        let id = metadata["id"].as_str().ok_or(IDENTITY_ERROR)?;
        if metadata["is_live"].as_bool() != Some(true) || !valid(reference, id) {
            return Err(IDENTITY_ERROR.into());
        }
        let uploader = if reference.provider == "soop" {
            let uploader = metadata["uploader_id"].as_str().ok_or(IDENTITY_ERROR)?;
            if !channel_id(uploader)
                || !uploader.eq_ignore_ascii_case(&reference.external_id)
                || station_id != Some(id)
            {
                return Err(IDENTITY_ERROR.into());
            }
            Some(uploader.to_owned())
        } else {
            None
        };
        if scheduled_id.is_some_and(|expected| expected != id) {
            return Err(IDENTITY_ERROR.into());
        }
        Ok(Self {
            id: id.into(),
            uploader,
        })
    }
    pub(super) fn id(&self) -> &str {
        &self.id
    }
    pub(super) fn append_receiver_filter(&self, args: &mut Vec<String>) {
        // A single AND filter: multiple CLI filters are OR alternatives. These
        // values contain no regex metacharacters, quotes, separators or escapes.
        let mut filter = format!("id ~= '\\A{}\\Z' & is_live", self.id);
        if let Some(uploader) = &self.uploader {
            filter.push_str(&format!(" & uploader_id ~= '\\A{uploader}\\Z'"));
        }
        args.extend(["--break-match-filters".into(), filter]);
    }
}

// An optional bad field must not make an otherwise valid old recording disappear.
pub(super) fn deserialize_optional<'de, D: Deserializer<'de>>(
    d: D,
) -> Result<Option<String>, D::Error> {
    let value = serde_json::Value::deserialize(d)?;
    Ok(value.as_str().filter(|v| v.len() <= 40).map(str::to_owned))
}
pub(super) fn sanitize(job: &mut MediaJob) {
    job.broadcast_id = job
        .broadcast_id
        .take()
        .filter(|id| valid(&job.reference, id));
    if let Some(item) = &mut job.library {
        item.broadcast_id = item.broadcast_id.take().filter(|id| {
            item.reference == job.reference
                && valid(&item.reference, id)
                && job.broadcast_id.as_ref() == Some(id)
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn soop() -> MediaRef {
        MediaRef {
            provider: "soop".into(),
            kind: "live".into(),
            external_id: "fixture_1".into(),
        }
    }
    #[test]
    fn preflight_requires_the_same_requested_station_and_scheduled_broadcast() {
        let good = serde_json::json!({"id":"123","uploader_id":"fixture_1","is_live":true});
        assert!(Binding::preflight(&soop(), &good, Some("123"), Some("123")).is_ok());
        assert!(Binding::preflight(&soop(), &good, Some("124"), None).is_err());
        assert!(Binding::preflight(&soop(), &good, Some("123"), Some("124")).is_err());
        for (key, value) in [
            ("uploader_id", serde_json::json!("other")),
            ("is_live", serde_json::json!(false)),
            ("id", serde_json::Value::Null),
        ] {
            let mut bad = good.clone();
            bad[key] = value;
            assert!(Binding::preflight(&soop(), &bad, Some("123"), None).is_err());
        }
    }
    #[test]
    fn receiver_filter_is_one_strict_and_argument_and_ids_cannot_inject_syntax() {
        let good = serde_json::json!({"id":"18446744073709551615","uploader_id":"fixture_1","is_live":true});
        let binding =
            Binding::preflight(&soop(), &good, Some("18446744073709551615"), None).unwrap();
        let mut args = vec!["--ignore-config".into()];
        binding.append_receiver_filter(&mut args);
        args.extend(["--".into(), "https://play.sooplive.co.kr/fixture_1".into()]);
        assert_eq!(args[1], "--break-match-filters");
        assert_eq!(
            args[2],
            "id ~= '\\A18446744073709551615\\Z' & is_live & uploader_id ~= '\\Afixture_1\\Z'"
        );
        assert_eq!(
            args.iter()
                .filter(|a| *a == "--break-match-filters")
                .count(),
            1
        );
        for id in [
            "0",
            "0123",
            "18446744073709551616",
            "123\n",
            "123' & id",
            "../123",
            "",
        ] {
            let mut bad = good.clone();
            bad["id"] = serde_json::json!(id);
            assert!(Binding::preflight(&soop(), &bad, Some(id), None).is_err());
        }
        let youtube = MediaRef {
            provider: "youtube".into(),
            kind: "live".into(),
            external_id: "abC_01-dEfG".into(),
        };
        assert!(Binding::preflight(
            &youtube,
            &serde_json::json!({"id":"abC_01-dEfG","is_live":true}),
            None,
            None
        )
        .is_ok());
        assert!(!valid(&youtube, "abC_01-dEfh"));
        let mut vod = youtube;
        vod.kind = "video".into();
        assert!(!valid(&vod, "abC_01-dEfG"));
    }

    #[test]
    #[ignore = "explicit pinned executable test; isolated infojson and loopback synthetic bytes only"]
    fn pinned_receiver_rejects_identity_before_synthetic_media_request() {
        use std::{
            fs,
            io::{Read, Write},
            net::{Shutdown, TcpListener},
            sync::{
                atomic::{AtomicBool, AtomicUsize, Ordering},
                Arc,
            },
            time::{Duration, Instant},
        };
        let repo = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .to_owned();
        let root = repo
            .join(".runtime/platform-broadcast-tests")
            .join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(&root).unwrap();
        let tool = repo.join(".runtime/platform-tools/bin/yt-dlp.exe");
        let cancel = Arc::new(AtomicBool::new(false));
        assert_eq!(
            super::super::storage::hash(&tool, &cancel).unwrap(),
            "66674953fe251b89f4d08c5f0e35e0728679bd67ab3d7d05c0562af101dd3e7a"
        );
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}/synthetic.mp4", listener.local_addr().unwrap());
        let count = Arc::new(AtomicUsize::new(0));
        let stopped = Arc::new(AtomicBool::new(false));
        struct Server(Arc<AtomicBool>, Option<std::thread::JoinHandle<()>>);
        impl Drop for Server {
            fn drop(&mut self) {
                self.0.store(true, Ordering::Release);
                if let Some(worker) = self.1.take() {
                    worker.join().unwrap();
                }
            }
        }
        let worker_stop = stopped.clone();
        let worker_count = count.clone();
        let server = Server(
            stopped,
            Some(std::thread::spawn(move || {
                while !worker_stop.load(Ordering::Acquire) {
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            stream
                                .set_write_timeout(Some(Duration::from_secs(1)))
                                .unwrap();
                            let mut request = [0u8; 4096];
                            let deadline = Instant::now() + Duration::from_secs(1);
                            let mut length = 0;
                            let mut complete = false;
                            while length < request.len() {
                                let remaining = deadline.saturating_duration_since(Instant::now());
                                if remaining.is_zero() {
                                    break;
                                }
                                stream.set_read_timeout(Some(remaining)).unwrap();
                                match stream.read(&mut request[length..]) {
                                    Ok(0) | Err(_) => break,
                                    Ok(read) => length += read,
                                }
                                if request[..length]
                                    .windows(4)
                                    .any(|bytes| bytes == b"\r\n\r\n")
                                {
                                    complete = true;
                                    break;
                                }
                            }
                            if !complete {
                                continue;
                            }
                            worker_count.fetch_add(1, Ordering::AcqRel);
                            let body = b"atsumi-synthetic-not-user-media";
                            let header=format!("HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",body.len());
                            let _ = stream.write_all(header.as_bytes());
                            let _ = stream.write_all(body);
                            let _ = stream.shutdown(Shutdown::Write);
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(10))
                        }
                        Err(_) => break,
                    }
                }
            })),
        );
        let binding = Binding::preflight(
            &soop(),
            &serde_json::json!({"id":"123","uploader_id":"fixture_1","is_live":true}),
            Some("123"),
            None,
        )
        .unwrap();
        for case in 0..7 {
            let mut info = serde_json::json!({"id":"123","uploader_id":"fixture_1","is_live":true,"title":"synthetic", "ext":"mp4","protocol":"http","url":url,"extractor":"soop:live","extractor_key":"AfreecaTVLive"});
            match case {
                1 => info["id"] = serde_json::json!("124"),
                2 => info["uploader_id"] = serde_json::json!("other"),
                3 => {
                    info.as_object_mut().unwrap().remove("uploader_id");
                }
                4 => info["is_live"] = serde_json::json!(false),
                5 => {
                    info.as_object_mut().unwrap().remove("id");
                }
                6 => {
                    info.as_object_mut().unwrap().remove("is_live");
                }
                _ => {}
            }
            let path = root.join(format!("case-{case}.json"));
            fs::write(&path, serde_json::to_vec(&info).unwrap()).unwrap();
            let mut args: Vec<String> = [
                "--ignore-config",
                "--no-plugin-dirs",
                "--no-cache-dir",
                "--no-remote-components",
                "--no-js-runtimes",
                "--no-playlist",
                "--no-cookies",
                "--no-cookies-from-browser",
                "--no-progress",
                "--no-warnings",
                "--no-part",
                "--retries",
                "0",
                "--socket-timeout",
                "2",
                "--proxy",
                "",
            ]
            .into_iter()
            .map(String::from)
            .collect();
            binding.append_receiver_filter(&mut args);
            args.extend([
                "--output".into(),
                "-".into(),
                "--load-info-json".into(),
                path.to_string_lossy().into_owned(),
            ]);
            let before = count.load(Ordering::Acquire);
            let output = super::super::process::run(
                &tool,
                &args,
                &root,
                &cancel,
                Duration::from_secs(12),
                65536,
            )
            .unwrap();
            if case == 0 {
                assert!(
                    output.success,
                    "matching synthetic fixture failed: stdout_bytes={} requests={} stderr={}",
                    output.stdout.len(),
                    count.load(Ordering::Acquire).saturating_sub(before),
                    String::from_utf8_lossy(&output.stderr)
                );
                assert_eq!(output.stdout, b"atsumi-synthetic-not-user-media");
                assert_eq!(count.load(Ordering::Acquire), before + 1);
            } else {
                assert!(!output.success, "mismatching fixture passed: {case}");
                assert!(output.stdout.is_empty(), "rejected receiver emitted bytes");
                assert_eq!(
                    count.load(Ordering::Acquire),
                    before,
                    "rejected receiver contacted media endpoint"
                );
            }
        }
        drop(server);
        eprintln!("broadcast binding offline: matching=1 rejected=6 synthetic_requests=1 public_media_requests=0");
    }
}
