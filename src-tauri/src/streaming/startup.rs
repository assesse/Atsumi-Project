//! CHZZK disk recovery is independent of the first Hitomi frame. The slot owns
//! initialization as well as the live service, so shutdown cannot miss a late
//! publication and factory reset cannot race a still-loading recording store.
use super::{browser::OfficialBrowser, browser_merge::MediaTools, model::StreamError};
use std::{
    path::PathBuf,
    sync::{Arc, Condvar, Mutex},
};
use tauri::Manager;

#[derive(Default)]
struct Slot<T> {
    value: Option<T>,
    initializing: bool,
    closed: bool,
}

impl<T> Slot<T> {
    fn publish(&mut self, value: T) -> Result<(), T> {
        if self.closed {
            Err(value)
        } else {
            self.value = Some(value);
            Ok(())
        }
    }
}

fn take_after_initialization<T>(inner: &(Mutex<Slot<T>>, Condvar)) -> Option<T> {
    let mut slot = inner.0.lock().unwrap_or_else(|e| e.into_inner());
    slot.closed = true;
    while slot.initializing {
        slot = inner.1.wait(slot).unwrap_or_else(|e| e.into_inner());
    }
    slot.value.take()
}

struct InitializationFinished<T>(Arc<(Mutex<Slot<T>>, Condvar)>);
impl<T> Drop for InitializationFinished<T> {
    fn drop(&mut self) {
        let mut slot = self.0 .0.lock().unwrap_or_else(|e| e.into_inner());
        slot.initializing = false;
        self.0 .1.notify_all();
    }
}

#[derive(Clone)]
pub(crate) struct DeferredBrowser {
    inner: Arc<(Mutex<Slot<OfficialBrowser>>, Condvar)>,
}

impl Default for DeferredBrowser {
    fn default() -> Self {
        Self {
            inner: Arc::new((
                Mutex::new(Slot {
                    value: None,
                    initializing: false,
                    closed: false,
                }),
                Condvar::new(),
            )),
        }
    }
}

impl DeferredBrowser {
    pub fn ready(value: Option<OfficialBrowser>) -> Self {
        Self {
            inner: Arc::new((
                Mutex::new(Slot {
                    value,
                    initializing: false,
                    closed: false,
                }),
                Condvar::new(),
            )),
        }
    }

    pub fn get(&self) -> Result<OfficialBrowser, StreamError> {
        let slot = self.inner.0.lock().unwrap_or_else(|e| e.into_inner());
        if slot.closed {
            return Err(StreamError::new(
                "APP_QUITTING",
                "앱 종료 중에는 녹화 기능을 사용할 수 없습니다.",
                false,
            ));
        }
        slot.value.clone().ok_or_else(|| if slot.initializing {
            StreamError::new("BROWSER_INITIALIZING", "녹화 기록을 준비하고 있습니다. 다른 탭은 계속 사용할 수 있습니다.", true)
        } else {
            StreamError::new("BROWSER_UNAVAILABLE", "공식 시청 녹화 저장소를 초기화하지 못했습니다. 저장 공간과 앱 로그를 확인해 주세요.", true)
        })
    }

    pub fn start(&self, app: tauri::AppHandle, data_dir: PathBuf, tools: MediaTools) {
        {
            let mut slot = self.inner.0.lock().unwrap_or_else(|e| e.into_inner());
            if slot.closed || slot.initializing || slot.value.is_some() {
                return;
            }
            slot.initializing = true;
        }
        let owner = self.clone();
        let spawn = std::thread::Builder::new().name("atsumi-recording-initialize".into()).spawn(move || {
            // Also releases shutdown waiters if publication/controller setup panics.
            let _finished = InitializationFinished(owner.inner.clone());
            crate::startup::mark("recordings_begin");
            let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(||
                OfficialBrowser::new_with_media_tools(data_dir.clone(), Some(tools))));
            let mut slot = owner.inner.0.lock().unwrap_or_else(|e| e.into_inner());
            match built {
                Ok(Ok(browser)) if !slot.closed => {
                    app.manage(super::replay::ReplayService::new(&data_dir, browser.capture_store()));
                    // The close flag and publication share this lock.
                    let _ = slot.publish(browser.clone());
                    // Only starts controllers; no network/disk wait under this lock.
                    if let Err(error) = browser.start_auto_recording(&app) {
                        tracing::warn!(code = %error.code, "automatic recording worker could not start");
                    }
                    crate::startup::mark("recordings_ready");
                }
                Ok(Ok(browser)) => {
                    // Keep initializing=true until the unpublished worker is joined.
                    drop(slot);
                    browser.shutdown_and_wait(&app);
                }
                Ok(Err(error)) => tracing::warn!(code = %error.code, "recording storage initialization failed"),
                Err(_) => tracing::error!("recording storage initializer panicked"),
            }
        });
        if spawn.is_err() {
            let mut slot = self.inner.0.lock().unwrap_or_else(|e| e.into_inner());
            slot.initializing = false;
            self.inner.1.notify_all();
            tracing::error!("could not spawn recording initializer");
        }
    }

    pub fn shutdown_and_wait(&self, app: &tauri::AppHandle) {
        let browser = take_after_initialization(&self.inner);
        // Publication has finished; no replay service can appear after this check.
        if let Some(replay) = app.try_state::<super::replay::ReplayService>() {
            replay.shutdown_and_wait();
        }
        if let Some(browser) = browser {
            browser.shutdown_and_wait(app);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn initializer_panic_releases_shutdown_waiter() {
        let inner = Arc::new((
            Mutex::new(Slot {
                value: Some(7),
                initializing: true,
                closed: false,
            }),
            Condvar::new(),
        ));
        let worker = inner.clone();
        assert!(std::thread::spawn(move || {
            let _finished = InitializationFinished(worker.clone());
            let _slot = worker.0.lock().unwrap();
            panic!("publication failed");
        })
        .join()
        .is_err());
        assert_eq!(take_after_initialization(&inner), Some(7));
    }
    #[test]
    fn shutdown_waits_for_initializer_and_rejects_its_late_service() {
        let inner = Arc::new((
            Mutex::new(Slot {
                value: None,
                initializing: true,
                closed: false,
            }),
            Condvar::new(),
        ));
        let closer = inner.clone();
        let done = std::thread::spawn(move || take_after_initialization(&closer));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            let mut slot = inner.0.lock().unwrap();
            if slot.closed {
                assert!(slot.publish(42).is_err());
                // An unpublished service must be cleaned up before this flag.
                slot.initializing = false;
                inner.1.notify_all();
                break;
            }
            drop(slot);
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert_eq!(done.join().unwrap(), None);
    }
    #[test]
    fn published_service_is_taken_exactly_once() {
        let inner = (
            Mutex::new(Slot {
                value: Some(42),
                initializing: false,
                closed: false,
            }),
            Condvar::new(),
        );
        assert_eq!(take_after_initialization(&inner), Some(42));
        assert_eq!(take_after_initialization(&inner), None);
        assert!(inner.0.lock().unwrap().publish(7).is_err());
    }
}
