use std::path::PathBuf;

use crate::application::{DownloadPipelineError, DownloadPipelineErrorCode, DownloadRootPicker};

#[derive(Debug, Default)]
pub struct WindowsFolderPicker;

impl WindowsFolderPicker {
    pub const fn new() -> Self {
        Self
    }
}

impl DownloadRootPicker for WindowsFolderPicker {
    fn pick_download_root(&self) -> Result<Option<PathBuf>, DownloadPipelineError> {
        pick_folder()
    }
}

#[cfg(windows)]
fn pick_folder() -> Result<Option<PathBuf>, DownloadPipelineError> {
    use windows::{
        core::HRESULT,
        Win32::{
            System::Com::{
                CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize,
                CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
            },
            UI::Shell::{
                FileOpenDialog, IFileOpenDialog, FOS_FORCEFILESYSTEM, FOS_PATHMUSTEXIST,
                FOS_PICKFOLDERS, SIGDN_FILESYSPATH,
            },
        },
    };

    struct ComApartment;
    impl Drop for ComApartment {
        fn drop(&mut self) {
            unsafe { CoUninitialize() };
        }
    }

    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED)
            .ok()
            .map_err(|_| picker_error("Windows could not initialize the folder picker"))?;
    }
    let _apartment = ComApartment;
    let dialog: IFileOpenDialog = unsafe {
        CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)
            .map_err(|_| picker_error("Windows could not create the folder picker"))?
    };
    let options = unsafe { dialog.GetOptions() }
        .map_err(|_| picker_error("Windows could not configure the folder picker"))?;
    unsafe {
        dialog.SetOptions(options | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST)
    }
    .map_err(|_| picker_error("Windows could not configure the folder picker"))?;

    if let Err(error) = unsafe { dialog.Show(None) } {
        // HRESULT_FROM_WIN32(ERROR_CANCELLED = 1223)
        if error.code() == HRESULT::from_win32(1223) {
            return Ok(None);
        }
        return Err(picker_error("The Windows folder picker could not be shown"));
    }

    let item = unsafe { dialog.GetResult() }
        .map_err(|_| picker_error("The selected folder could not be read"))?;
    let display_name = unsafe { item.GetDisplayName(SIGDN_FILESYSPATH) }
        .map_err(|_| picker_error("The selected folder is not a filesystem path"))?;
    let path = unsafe { display_name.to_string() }
        .map(PathBuf::from)
        .map_err(|_| picker_error("The selected folder path could not be decoded"));
    unsafe { CoTaskMemFree(Some(display_name.0.cast())) };
    path.map(Some)
}

#[cfg(not(windows))]
fn pick_folder() -> Result<Option<PathBuf>, DownloadPipelineError> {
    Err(picker_error(
        "The native download folder picker is available only on Windows",
    ))
}

fn picker_error(message: &'static str) -> DownloadPipelineError {
    DownloadPipelineError::new(DownloadPipelineErrorCode::RootUnavailable, message, false)
}
