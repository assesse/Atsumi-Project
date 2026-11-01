//! Keep browser chrome out of the application, without taking over DOM menus.
//! Native edit commands retain WebView2's selection, clipboard and enabled state.

pub(crate) fn init() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri::plugin::Builder::new("context-menu-policy")
        .on_webview_ready(|view| {
            #[cfg(windows)]
            install(view);
            #[cfg(not(windows))]
            let _ = view;
        })
        .build()
}

#[cfg(any(windows, test))]
fn keep_edit_command(name: &str, editable: bool, selection: bool) -> bool {
    if editable {
        matches!(
            name,
            "undo"
                | "redo"
                | "cut"
                | "copy"
                | "paste"
                | "pasteAndMatchStyle"
                | "pasteAsPlainText"
                | "delete"
                | "selectAll"
        )
    } else {
        selection && name == "copy"
    }
}

#[cfg(windows)]
fn install(view: tauri::Webview<tauri::Wry>) {
    use webview2_com::{
        ContextMenuRequestedEventHandler, Microsoft::Web::WebView2::Win32::ICoreWebView2_11,
    };
    use windows::core::Interface;

    let label = view.label().to_owned();
    let dispatch_label = label.clone();
    if let Err(error) = view.with_webview(move |platform| unsafe {
        let Ok(base) = platform.controller().CoreWebView2() else {
            tracing::warn!(webview = %label, "native context menu policy could not access WebView2");
            return;
        };
        let result = (|| -> windows::core::Result<()> {
            let core = base.cast::<ICoreWebView2_11>()?;
            let callback_label = label.clone();
            let mut token = 0;
            core.add_ContextMenuRequested(
                &ContextMenuRequestedEventHandler::create(Box::new(move |_, args| {
                    if let Some(args) = args {
                        if let Err(error) = filter_native_menu(&args) {
                            // Keep a partial/unknown menu hidden. Never inspect selected
                            // text, passwords, clipboard data or URLs in this callback.
                            let _ = args.SetHandled(true);
                            tracing::warn!(webview = %callback_label, %error, "native context menu was suppressed after a filter error");
                        }
                    }
                    Ok(())
                })),
                &mut token,
            )?;
            Ok(())
        })();
        if let Err(error) = result {
            // Older runtimes cannot filter individual commands. Suppress browser
            // chrome there; native keyboard editing and DOM menus still work.
            let fallback = base.Settings().and_then(|settings| settings.SetAreDefaultContextMenusEnabled(false));
            tracing::warn!(webview = %label, %error, fallback_succeeded = fallback.is_ok(), "native edit-menu filtering unavailable; default-menu suppression fallback");
        } else {
            tracing::debug!(webview = %label, "native context menu policy installed");
        }
    }) {
        tracing::warn!(webview = %dispatch_label, %error, "native context menu policy could not be scheduled");
    }
}

#[cfg(windows)]
unsafe fn filter_native_menu(
    args: &webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2ContextMenuRequestedEventArgs,
) -> windows::core::Result<()> {
    use webview2_com::{
        take_pwstr, Microsoft::Web::WebView2::Win32::COREWEBVIEW2_CONTEXT_MENU_ITEM_KIND_COMMAND,
    };
    use windows::core::{BOOL, PWSTR};

    // DOM handlers already had the opportunity to prevent the default menu.
    // Start hidden so any failed COM read/removal leaves no unrelated commands.
    args.SetHandled(true)?;
    let target = args.ContextMenuTarget()?;
    let (mut editable, mut selection) = (BOOL(0), BOOL(0));
    target.IsEditable(&mut editable)?;
    target.HasSelection(&mut selection)?;
    if !editable.as_bool() && !selection.as_bool() {
        return Ok(());
    }
    let items = args.MenuItems()?;
    let mut count = 0;
    items.Count(&mut count)?;
    if count > 256 {
        return Ok(());
    }
    let mut retained = 0;
    for index in (0..count).rev() {
        let item = items.GetValueAtIndex(index)?;
        let mut name = PWSTR::null();
        let name_result = item.Name(&mut name);
        let name = take_pwstr(name);
        name_result?;
        let mut kind = Default::default();
        item.Kind(&mut kind)?;
        if kind == COREWEBVIEW2_CONTEXT_MENU_ITEM_KIND_COMMAND
            && keep_edit_command(&name, editable.as_bool(), selection.as_bool())
        {
            retained += 1;
        } else {
            items.RemoveValueAtIndex(index)?;
        }
    }
    if retained > 0 {
        args.SetHandled(false)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::keep_edit_command;

    const BROWSER_COMMANDS: &[&str] = &[
        "back",
        "reload",
        "saveAs",
        "print",
        "webCapture",
        "inspectElement",
        "openLinkInNewWindow",
        "copyLink",
        "copyImage",
        "saveImageAs",
        "searchWeb",
        "extension",
        "custom",
        "unknownFutureCommand",
    ];

    #[test]
    fn unrelated_browser_actions_never_leak_into_an_edit_or_selection_menu() {
        for editable in [false, true] {
            for selection in [false, true] {
                for command in BROWSER_COMMANDS {
                    assert!(
                        !keep_edit_command(command, editable, selection),
                        "{command}"
                    );
                }
            }
        }
    }

    #[test]
    fn blank_surfaces_have_no_menu_and_selected_text_only_keeps_copy() {
        for name in ["copy", "cut", "paste", "delete", "selectAll"] {
            assert!(!keep_edit_command(name, false, false));
            assert_eq!(keep_edit_command(name, false, true), name == "copy");
        }
    }

    #[test]
    fn editable_fields_keep_native_editing_even_without_a_selection() {
        for selection in [false, true] {
            for name in [
                "undo",
                "redo",
                "cut",
                "copy",
                "paste",
                "pasteAndMatchStyle",
                "pasteAsPlainText",
                "delete",
                "selectAll",
            ] {
                assert!(keep_edit_command(name, true, selection), "{name}");
            }
        }
    }
}
