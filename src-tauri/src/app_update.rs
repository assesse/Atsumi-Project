//! Update installation has its own lifetime, even when no database/AppState exists.
//! Only a terminal startup failure may use the bootstrap path. Ready applications
//! retain their atomic managed-work reservation and normal shutdown protocol.
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Manager, Runtime, Webview};

use crate::{
    interface::AppState,
    startup::{Phase, Startup},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Reservation {
    FailedStartup,
    Application,
}

#[derive(Default)]
pub(crate) struct UpdateGate {
    reservation: Mutex<Option<Reservation>>,
}

fn lock_error() -> String {
    "업데이트 작업 상태를 확인하지 못했습니다. 다시 시도해 주세요.".into()
}

impl UpdateGate {
    fn reserve(
        &self,
        phase: Phase,
        has_backend: bool,
        reserve_backend: impl FnOnce() -> Result<(), String>,
    ) -> Result<(), String> {
        let mut reservation = self.reservation.lock().map_err(|_| lock_error())?;
        if reservation.is_some() {
            return Err("이미 업데이트를 준비하고 있습니다.".into());
        }
        let next = match (phase, has_backend) {
            (Phase::Failed, false) => Reservation::FailedStartup,
            (Phase::Ready | Phase::Failed, true) => {
                reserve_backend()?;
                Reservation::Application
            }
            (Phase::Starting | Phase::Cancelling, _) => {
                return Err("앱 데이터 준비·정리가 끝난 뒤 업데이트를 설치해 주세요.".into());
            }
            _ => return Err(lock_error()),
        };
        *reservation = Some(next);
        Ok(())
    }

    fn release(&self, release_backend: impl FnOnce()) -> Result<(), String> {
        let mut reservation = self.reservation.lock().map_err(|_| lock_error())?;
        if *reservation == Some(Reservation::Application) {
            release_backend();
        }
        *reservation = None;
        Ok(())
    }
}

fn require_main<R: Runtime>(webview: &Webview<R>) -> Result<(), String> {
    if webview.label() == "main" {
        Ok(())
    } else {
        Err("앱의 기본 창에서만 업데이트할 수 있습니다.".into())
    }
}

#[tauri::command]
pub(crate) async fn app_update_reserve<R: Runtime>(
    app: AppHandle<R>,
    webview: Webview<R>,
) -> Result<(), String> {
    require_main(&webview)?;
    tauri::async_runtime::spawn_blocking(move || {
        let startup = app.state::<Arc<Startup>>();
        let backend = app.try_state::<AppState>();
        app.state::<UpdateGate>()
            .reserve(startup.phase(), backend.is_some(), || {
                backend
                    .as_ref()
                    .ok_or_else(lock_error)?
                    .reserve_update()
                    .map_err(|e| e.message)
            })
    })
    .await
    .map_err(|_| lock_error())?
}

#[tauri::command]
pub(crate) async fn app_update_release<R: Runtime>(
    app: AppHandle<R>,
    webview: Webview<R>,
) -> Result<(), String> {
    require_main(&webview)?;
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<UpdateGate>().release(|| {
            if let Some(backend) = app.try_state::<AppState>() {
                backend.release_update();
            }
        })
    })
    .await
    .map_err(|_| lock_error())?
}

pub(crate) fn is_reserved(app: &AppHandle) -> Result<bool, String> {
    let gate = app.state::<UpdateGate>();
    let reservation = gate.reservation.lock().map_err(|_| lock_error())?;
    Ok(match *reservation {
        Some(Reservation::FailedStartup) => {
            app.state::<Arc<Startup>>().phase() == Phase::Failed
                && app.try_state::<AppState>().is_none()
        }
        Some(Reservation::Application) => app
            .try_state::<AppState>()
            .is_some_and(|state| state.update_reserved()),
        None => false,
    })
}

pub(crate) fn finish_portable_update(app: &AppHandle) -> Result<(), String> {
    let gate = app.state::<UpdateGate>();
    let reservation = gate.reservation.lock().map_err(|_| lock_error())?;
    match *reservation {
        Some(Reservation::FailedStartup)
            if app.state::<Arc<Startup>>().phase() == Phase::Failed
                && app.try_state::<AppState>().is_none() =>
        {
            // Startup failure is published only after the initializer returns.
            // No workers were resumed and there is no database-backed quit path.
            app.exit(0);
            Ok(())
        }
        Some(Reservation::Application) => app
            .try_state::<AppState>()
            .ok_or_else(lock_error)?
            .finish_portable_update(app.clone()),
        _ => Err("업데이트 작업 잠금이 필요합니다.".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn only_terminal_failure_can_reserve_without_a_backend() {
        let gate = UpdateGate::default();
        for phase in [Phase::Starting, Phase::Cancelling, Phase::Ready] {
            assert!(gate.reserve(phase, false, || panic!("no backend")).is_err());
        }
        gate.reserve(Phase::Failed, false, || panic!("database must not be used"))
            .unwrap();
        assert!(gate.reserve(Phase::Failed, false, || Ok(())).is_err());
        gate.release(|| panic!("no backend release")).unwrap();
        gate.reserve(Phase::Failed, false, || panic!("no backend"))
            .unwrap();
    }

    #[test]
    fn initialized_or_partly_published_backend_keeps_the_active_work_barrier() {
        for phase in [Phase::Ready, Phase::Failed] {
            let gate = UpdateGate::default();
            let error = gate
                .reserve(phase, true, || Err("active recording".into()))
                .unwrap_err();
            assert_eq!(error, "active recording");
            assert!(gate.reservation.lock().unwrap().is_none());
            gate.reserve(phase, true, || Ok(())).unwrap();
            let released = AtomicUsize::new(0);
            gate.release(|| {
                released.fetch_add(1, Ordering::SeqCst);
            })
            .unwrap();
            gate.release(|| {
                released.fetch_add(1, Ordering::SeqCst);
            })
            .unwrap();
            assert_eq!(released.load(Ordering::SeqCst), 1);
        }
    }

    #[test]
    fn unfinished_initialization_never_calls_backend_even_if_state_was_published() {
        for phase in [Phase::Starting, Phase::Cancelling] {
            assert!(UpdateGate::default()
                .reserve(phase, true, || panic!("unsafe install"))
                .is_err());
        }
    }

    fn request<R: Runtime>(app: &tauri::App<R>, cmd: &str) -> tauri::webview::InvokeRequest {
        tauri::webview::InvokeRequest {
            cmd: cmd.into(),
            callback: tauri::ipc::CallbackFn(0),
            error: tauri::ipc::CallbackFn(1),
            url: "http://tauri.localhost".parse().unwrap(),
            body: tauri::ipc::InvokeBody::default(),
            headers: Default::default(),
            invoke_key: app.handle().invoke_key().to_string(),
        }
    }

    #[test]
    fn real_ipc_dispatch_reserves_releases_and_retries_after_failure_without_app_state() {
        use tauri::test::{get_ipc_response, mock_builder, mock_context, noop_assets};
        let startup = Arc::new(Startup::default());
        let app = mock_builder()
            .manage(startup.clone())
            .manage(UpdateGate::default())
            .invoke_handler(crate::startup::gate(tauri::generate_handler![
                app_update_reserve,
                app_update_release
            ]))
            .build(mock_context(noop_assets()))
            .unwrap();
        let window = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .unwrap();
        assert!(app.try_state::<AppState>().is_none());
        // Must fail immediately, not sit behind the startup IPC queue.
        assert!(get_ipc_response(&window, request(&app, "app_update_reserve")).is_err());
        startup.fail();
        for _ in 0..2 {
            get_ipc_response(&window, request(&app, "app_update_reserve")).unwrap();
            assert!(get_ipc_response(&window, request(&app, "app_update_reserve")).is_err());
            get_ipc_response(&window, request(&app, "app_update_release")).unwrap();
        }
        assert!(get_ipc_response(&window, request(&app, "settings_get")).is_err());
        let foreign = tauri::WebviewWindowBuilder::new(&app, "foreign", Default::default())
            .build()
            .unwrap();
        assert!(get_ipc_response(&foreign, request(&app, "app_update_reserve")).is_err());
        assert!(get_ipc_response(&foreign, request(&app, "app_update_release")).is_err());
    }

    #[test]
    fn update_discovery_bypasses_startup_queue_but_data_commands_do_not() {
        use tauri::test::{get_ipc_response, mock_builder, mock_context, noop_assets};
        let startup = Arc::new(Startup::default());
        let app = mock_builder()
            .manage(startup.clone())
            .invoke_handler(crate::startup::gate(|invoke| {
                invoke.resolver.resolve("independent");
                true
            }))
            .build(mock_context(noop_assets()))
            .unwrap();
        let window = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .unwrap();
        for failed in [false, true] {
            if failed {
                startup.fail();
            }
            let response = get_ipc_response(&window, request(&app, "app_update_mode")).unwrap();
            assert_eq!(response.deserialize::<String>().unwrap(), "independent");
        }
        assert!(get_ipc_response(&window, request(&app, "settings_get")).is_err());
    }
}
