use crate::domain::{DownloadJobProjection, JobState};
use std::{
    sync::{mpsc, OnceLock},
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Notice {
    QueueDone,
    QueueNeedsAttention,
    RecordingSaved,
    PartialRecordingSaved,
}

impl Notice {
    fn text(self) -> (&'static str, &'static str) {
        match self {
            Self::QueueDone => (
                "다운로드 대기 큐 처리 완료",
                "다운로드와 후처리가 끝났습니다.",
            ),
            Self::QueueNeedsAttention => (
                "다운로드 대기 큐 처리 종료",
                "활동 기록에 확인할 작업이 있습니다.",
            ),
            Self::RecordingSaved => ("방송 녹화 저장 완료", "녹화 목록에서 재생할 수 있습니다."),
            Self::PartialRecordingSaved => (
                "방송 녹화 일부 저장",
                "저장된 구간을 녹화 목록에서 재생할 수 있습니다.",
            ),
        }
    }
}

static SENDER: OnceLock<mpsc::Sender<Notice>> = OnceLock::new();

pub(crate) fn install(identifier: &str) {
    if identifier != "local.atsumi.next" || SENDER.get().is_some() {
        return;
    }
    let (tx, rx) = mpsc::channel();
    let worker = std::thread::Builder::new()
        .name("atsumi-windows-notifications".into())
        .spawn(move || {
            for notice in rx {
                #[cfg(windows)]
                if let Err(error) = desktop::show(notice) {
                    tracing::warn!(%error, "Windows completion notification unavailable");
                }
                #[cfg(not(windows))]
                let _ = notice;
            }
        });
    match worker {
        Ok(_) => {
            let _ = SENDER.set(tx);
        }
        Err(error) => tracing::warn!(%error, "notification worker unavailable"),
    }
}

pub(crate) fn send(notice: Notice) {
    if let Some(tx) = SENDER.get() {
        let _ = tx.send(notice);
    }
}

pub(crate) fn recording_saved(partial: bool) {
    send(if partial {
        Notice::PartialRecordingSaved
    } else {
        Notice::RecordingSaved
    });
}

#[derive(Default)]
struct QueueWave {
    armed: bool,
    processed: bool,
    needs_attention: bool,
}

impl QueueWave {
    fn observe(&mut self, state: JobState) -> bool {
        if state.is_active() {
            self.armed = true;
            return false;
        }
        if !self.armed {
            return false;
        }
        match state {
            JobState::Completed | JobState::Quarantined => self.processed = true,
            JobState::ReviewRequired | JobState::Failed => {
                self.processed = true;
                self.needs_attention = true;
            }
            JobState::Interrupted => self.needs_attention = true,
            _ => {}
        }
        true
    }
    fn settle(&mut self, active: u64) -> Option<Notice> {
        if active != 0 {
            return None;
        }
        let done = std::mem::take(self);
        (done.armed && done.processed).then_some(if done.needs_attention {
            Notice::QueueNeedsAttention
        } else {
            Notice::QueueDone
        })
    }
}

/// Do not poll an idle queue or scan the database on byte-progress events.
/// Only a terminal event arms a short reconciliation against the durable queue.
pub(crate) fn forward_downloads(
    events: mpsc::Receiver<DownloadJobProjection>,
    active_count: impl Fn() -> Option<u64>,
    publish: impl Fn(DownloadJobProjection),
) {
    let mut wave = QueueWave {
        armed: active_count().is_some_and(|n| n > 0),
        ..Default::default()
    };
    let mut deadline: Option<Instant> = None;
    loop {
        if deadline.is_some_and(|when| Instant::now() >= when) {
            deadline = None;
            if let Some(n) = active_count() {
                if let Some(notice) = wave.settle(n) {
                    send(notice);
                }
            } else {
                deadline = Some(Instant::now() + Duration::from_secs(5));
            }
        }
        let event = match deadline {
            None => match events.recv() {
                Ok(v) => v,
                Err(_) => break,
            },
            Some(when) => match events.recv_timeout(when.saturating_duration_since(Instant::now()))
            {
                Ok(v) => v,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            },
        };
        if wave.observe(event.download.state) && deadline.is_none() {
            deadline = Some(Instant::now() + Duration::from_millis(900));
        }
        publish(event);
    }
}

#[cfg(windows)]
mod desktop {
    use super::Notice;
    use std::sync::OnceLock;
    use windows::{
        core::HSTRING,
        Data::Xml::Dom::XmlDocument,
        Win32::System::WinRT::{RoInitialize, RoUninitialize, RO_INIT_MULTITHREADED},
        UI::Notifications::{NotificationSetting, ToastNotification, ToastNotificationManager},
    };
    use winreg::{enums::HKEY_CURRENT_USER, RegKey};

    pub(super) const ID: &str = if cfg!(debug_assertions) {
        "local.atsumi.next.debug"
    } else {
        "local.atsumi.next"
    };

    // Unpackaged builds also need an identity when started from a portable
    // directory or the development launcher. Never change Windows' preferences.
    fn register() -> std::io::Result<()> {
        static READY: OnceLock<()> = OnceLock::new();
        if READY.get().is_none() {
            let root = RegKey::predef(HKEY_CURRENT_USER);
            let (key, _) = root.create_subkey(format!(r"Software\Classes\AppUserModelId\{ID}"))?;
            key.set_value("DisplayName", &"Atsumi")?;
            let _ = READY.set(());
        }
        Ok(())
    }

    pub(super) fn show(notice: Notice) -> Result<(), Box<dyn std::error::Error>> {
        let (title, body) = notice.text();
        deliver(title, body).map(|_| ())
    }

    fn deliver(title: &str, body: &str) -> Result<Option<HSTRING>, Box<dyn std::error::Error>> {
        register()?;
        // This worker owns the matching WinRT initialization on its own thread.
        unsafe {
            RoInitialize(RO_INIT_MULTITHREADED)?;
        }
        struct Apartment;
        impl Drop for Apartment {
            fn drop(&mut self) {
                unsafe {
                    RoUninitialize();
                }
            }
        }
        let _apartment = Apartment;
        let notifier = ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(ID))?;
        // For a newly registered desktop sender Windows may return ERROR_NOT_FOUND
        // until its first toast is submitted. Show still enforces all system settings.
        if notifier.Setting().is_ok_and(|setting| setting != NotificationSetting::Enabled) {
            tracing::info!("Windows has disabled Atsumi notification delivery");
            return Ok(None);
        }
        let xml = XmlDocument::new()?;
        xml.LoadXml(&HSTRING::from("<toast><visual><binding template=\"ToastGeneric\"><text/><text/></binding></visual></toast>"))?;
        let nodes = xml.GetElementsByTagName(&HSTRING::from("text"))?;
        nodes.Item(0)?.SetInnerText(&HSTRING::from(title))?;
        nodes.Item(1)?.SetInnerText(&HSTRING::from(body))?;
        let toast = ToastNotification::CreateToastNotification(&xml)?;
        let tag = HSTRING::from(uuid::Uuid::new_v4().simple().to_string());
        toast.SetTag(&tag)?;
        toast.SetGroup(&HSTRING::from("work"))?;
        notifier.Show(&toast)?;
        Ok(Some(tag))
    }

    #[test]
    #[ignore = "sends one clearly labelled local Windows notification"]
    fn windows_shell_receives_atsumi_probe() -> Result<(), Box<dyn std::error::Error>> {
        let tag = deliver(
            "Atsumi 알림 테스트",
            "작업 완료 알림의 Windows 수신을 확인하고 있습니다.",
        )?
        .ok_or("Atsumi notifications disabled in Windows")?;
        unsafe {
            RoInitialize(RO_INIT_MULTITHREADED)?;
        }
        let result = (|| -> windows::core::Result<bool> {
            let history = ToastNotificationManager::History()?;
            let id = HSTRING::from(ID);
            let mut found = false;
            for _ in 0..20 {
                std::thread::sleep(std::time::Duration::from_millis(300));
                let own = history.GetHistoryWithId(&id)?;
                if (0..own.Size()?)
                    .any(|i| own.GetAt(i).and_then(|t| t.Tag()).ok().as_ref() == Some(&tag))
                {
                    found = true;
                    break;
                }
            }
            history.RemoveGroupedTagWithId(&tag, &HSTRING::from("work"), &id)?;
            Ok(found)
        })();
        unsafe {
            RoUninitialize();
        }
        assert!(
            result?,
            "Windows did not confirm the tagged Atsumi notification"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_settled_work_notifies_once() {
        let mut wave = QueueWave::default();
        assert!(!wave.observe(JobState::Completed));
        assert_eq!(wave.settle(0), None);
        for state in [
            JobState::Queued,
            JobState::Downloading,
            JobState::Hashing,
            JobState::RetryWait,
            JobState::Verifying,
        ] {
            assert!(!wave.observe(state));
        }
        assert!(wave.observe(JobState::Completed));
        assert_eq!(wave.settle(1), None);
        assert!(wave.observe(JobState::ReviewRequired));
        assert_eq!(wave.settle(0), Some(Notice::QueueNeedsAttention));
        assert_eq!(wave.settle(0), None);
        wave.observe(JobState::Queued);
        wave.observe(JobState::Cancelled);
        assert_eq!(wave.settle(0), None);
        wave.observe(JobState::Hashing);
        wave.observe(JobState::Completed);
        assert_eq!(wave.settle(0), Some(Notice::QueueDone));
    }
}
