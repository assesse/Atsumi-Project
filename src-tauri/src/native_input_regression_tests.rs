//! Windows-only regression tests for the vendored Tao input deadlock fix.
//!
//! Register this test-only module as `platform_impl` in lib.rs so the unchanged
//! upstream MinimalIme source resolves its private ProcResult import. Its actual
//! state machine is compiled below; none of these tests opens a window or sends
//! input to the user's desktop. Private keyboard queue peeks are additionally
//! protected by source-structure assertions against the code Cargo builds.

pub(crate) mod platform {
    pub(crate) mod event_loop {
        use windows::Win32::Foundation::LRESULT;

        // Only the result carrier is substituted. MinimalIme's parsing/state
        // transitions come directly from the production vendored source.
        #[derive(Debug)]
        pub(crate) enum ProcResult {
            DefSubclassProc,
            Value(LRESULT),
        }
    }
}

#[path = "../vendor/tao-0.35.3/src/platform_impl/windows/minimal_ime.rs"]
#[rustfmt::skip]
mod upstream_minimal_ime;

use platform::event_loop::ProcResult;
use upstream_minimal_ime::{is_msg_ime_related, MinimalIme};
use windows::Win32::{
    Foundation::WPARAM,
    UI::WindowsAndMessaging::{
        WM_CHAR, WM_IME_CHAR, WM_IME_COMPOSITION, WM_IME_COMPOSITIONFULL, WM_IME_ENDCOMPOSITION,
        WM_IME_STARTCOMPOSITION, WM_KEYDOWN, WM_KEYUP, WM_PAINT, WM_SETFOCUS, WM_SYSCHAR,
        WM_SYSKEYDOWN, WM_SYSKEYUP,
    },
};

const EVENT_LOOP: &str =
    include_str!("../vendor/tao-0.35.3/src/platform_impl/windows/event_loop.rs");
const KEYBOARD: &str = include_str!("../vendor/tao-0.35.3/src/platform_impl/windows/keyboard.rs");
const IME: &str = include_str!("../vendor/tao-0.35.3/src/platform_impl/windows/minimal_ime.rs");

fn deliver(ime: &mut MinimalIme, message: u32, code_unit: u16, more: bool) -> Option<String> {
    let mut result = ProcResult::DefSubclassProc;
    let text = ime.process_message(message, WPARAM(usize::from(code_unit)), more, &mut result);
    if matches!(message, WM_CHAR | WM_SYSCHAR) {
        assert!(matches!(result, ProcResult::Value(value) if value.0 == 0));
    } else {
        assert!(matches!(result, ProcResult::DefSubclassProc));
    }
    text
}

#[test]
fn ordinary_bmp_chars_keep_immediate_delivery_without_an_ime_commit() {
    let mut ime = MinimalIme::default();
    for (message, character, more) in [
        (WM_CHAR, 'A', false),
        (WM_SYSCHAR, '한', true),
        (WM_CHAR, '글', false),
    ] {
        // A pre-peeked following character must not incorrectly start an IME
        // transaction for ordinary character events.
        assert_eq!(
            deliver(&mut ime, message, character as u16, more),
            Some(character.to_string())
        );
    }
}

#[test]
fn committed_ime_text_waits_for_the_prepeeked_last_character_once() {
    let mut ime = MinimalIme::default();
    assert_eq!(deliver(&mut ime, WM_IME_ENDCOMPOSITION, 0, false), None);
    assert_eq!(deliver(&mut ime, WM_CHAR, '한' as u16, true), None);
    assert_eq!(deliver(&mut ime, WM_SYSCHAR, '글' as u16, true), None);
    assert_eq!(
        deliver(&mut ime, WM_CHAR, '!' as u16, false),
        Some("한글!".into())
    );
    // The next normal character cannot repeat the completed commit buffer.
    assert_eq!(
        deliver(&mut ime, WM_CHAR, 'x' as u16, false),
        Some("x".into())
    );
}

#[test]
fn committed_utf16_surrogates_survive_char_and_syschar_boundaries() {
    for (first, second) in [
        (WM_CHAR, WM_CHAR),
        (WM_CHAR, WM_SYSCHAR),
        (WM_SYSCHAR, WM_CHAR),
        (WM_SYSCHAR, WM_SYSCHAR),
    ] {
        let mut ime = MinimalIme::default();
        assert_eq!(deliver(&mut ime, WM_IME_ENDCOMPOSITION, 0, false), None);
        assert_eq!(deliver(&mut ime, first, 0xd83d, true), None);
        assert_eq!(deliver(&mut ime, second, 0xde42, false), Some("🙂".into()));
        assert_eq!(
            deliver(&mut ime, WM_CHAR, 'z' as u16, true),
            Some("z".into())
        );
    }
}

#[test]
fn invalid_utf16_does_not_poison_the_next_ime_commit() {
    for invalid in [
        vec![0xd83d],
        vec![0xde42],
        vec![0xd83d, 0xd83d],
        vec![0xde42, 0xd83d],
    ] {
        let mut ime = MinimalIme::default();
        deliver(&mut ime, WM_IME_ENDCOMPOSITION, 0, false);
        for (index, &unit) in invalid.iter().enumerate() {
            assert_eq!(
                deliver(&mut ime, WM_CHAR, unit, index + 1 < invalid.len()),
                None
            );
        }
        deliver(&mut ime, WM_IME_ENDCOMPOSITION, 0, false);
        assert_eq!(
            deliver(&mut ime, WM_CHAR, '가' as u16, false),
            Some("가".into())
        );
    }
}

#[test]
fn key_press_release_messages_do_not_flush_or_corrupt_pending_ime_units() {
    for message in [WM_KEYDOWN, WM_SYSKEYDOWN, WM_KEYUP, WM_SYSKEYUP] {
        let mut ime = MinimalIme::default();
        deliver(&mut ime, WM_IME_ENDCOMPOSITION, 0, false);
        assert_eq!(deliver(&mut ime, WM_CHAR, 0xd83d, true), None);
        assert_eq!(deliver(&mut ime, message, 0x41, false), None);
        assert_eq!(deliver(&mut ime, WM_CHAR, 0xde42, false), Some("🙂".into()));
    }
}

#[test]
fn ime_routing_retains_composition_and_character_events_only() {
    for message in [
        WM_IME_COMPOSITION,
        WM_IME_COMPOSITIONFULL,
        WM_IME_STARTCOMPOSITION,
        WM_IME_ENDCOMPOSITION,
        WM_IME_CHAR,
        WM_CHAR,
        WM_SYSCHAR,
    ] {
        assert!(
            is_msg_ime_related(message),
            "IME route lost message {message:#x}"
        );
    }
    for message in [
        WM_KEYDOWN,
        WM_KEYUP,
        WM_SYSKEYDOWN,
        WM_SYSKEYUP,
        WM_SETFOCUS,
        WM_PAINT,
    ] {
        assert!(
            !is_msg_ime_related(message),
            "non-IME message routed: {message:#x}"
        );
    }
}

/// Remove comments and quoted contents before inspecting braces/calls. This is
/// deliberately a small source-contract guard, not a claim to type-check Rust.
/// Function/closure extraction below is limited to the vendored input routines.
fn code_only(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut result = bytes.to_vec();
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at..].starts_with(b"//") {
            let start = at;
            while at < bytes.len() && bytes[at] != b'\n' {
                at += 1;
            }
            result[start..at].fill(b' ');
        } else if bytes[at..].starts_with(b"/*") {
            let start = at;
            at += 2;
            let mut depth = 1;
            while at < bytes.len() && depth > 0 {
                if bytes[at..].starts_with(b"/*") {
                    depth += 1;
                    at += 2;
                } else if bytes[at..].starts_with(b"*/") {
                    depth -= 1;
                    at += 2;
                } else {
                    at += 1;
                }
            }
            assert_eq!(depth, 0, "unterminated Rust block comment");
            result[start..at].fill(b' ');
        } else if bytes[at] == b'"' {
            let start = at;
            at += 1;
            while at < bytes.len() {
                if bytes[at] == b'\\' {
                    at += 2;
                } else if bytes[at] == b'"' {
                    at += 1;
                    break;
                } else {
                    at += 1;
                }
            }
            assert!(at <= bytes.len(), "unterminated Rust string");
            result[start..at].fill(b' ');
        } else {
            at += 1;
        }
    }
    String::from_utf8(result).expect("ASCII replacements preserve UTF-8 outside comments/strings")
}

fn block_after<'a>(source: &'a str, marker: &str) -> &'a str {
    let marker_at = source
        .find(marker)
        .unwrap_or_else(|| panic!("missing production code: {marker}"));
    let open = marker_at
        + source[marker_at..]
            .find('{')
            .expect("expected a Rust block");
    let mut depth = 0;
    for (offset, byte) in source.as_bytes()[open..].iter().enumerate() {
        match byte {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return &source[open + 1..open + offset];
                }
            }
            _ => {}
        }
    }
    panic!("unclosed production block: {marker}");
}

fn compact(source: &str) -> String {
    source
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect()
}

#[test]
fn keyboard_and_ime_state_machines_never_peek_or_dispatch_messages_under_their_guards() {
    for (name, source) in [("keyboard", KEYBOARD), ("IME", IME)] {
        let code = code_only(source);
        for forbidden in [
            "PeekMessageW(",
            "PeekMessageA(",
            "GetMessageW(",
            "DispatchMessageW(",
        ] {
            assert!(!compact(&code).contains(forbidden), "{name} processing may reenter Win32 while its caller holds an input mutex: {forbidden}");
        }
    }
    let keyboard = compact(&code_only(KEYBOARD));
    assert!(keyboard.contains("next_key_message:Option<MSG>"));
    assert!(keyboard.contains("ifletSome(next_msg)=next_key_message"));
    assert!(compact(&code_only(IME)).contains("more_char_coming:bool"));
}

#[test]
fn actual_callbacks_take_snapshots_before_locking_and_dispatch_after_unlocking() {
    assert_input_lock_boundaries(EVENT_LOOP);
}

fn assert_input_lock_boundaries(source: &str) {
    let code = code_only(source);
    let keyboard = block_after(&code, "let keyboard_callback = ||");
    let snapshot = keyboard.find("next_key_message_for_keyboard(").unwrap();
    let critical = block_after(keyboard, "let events =");
    let critical_start = keyboard.find(critical).unwrap();
    assert!(
        snapshot < critical_start,
        "keyboard peek moved inside the input lock scope"
    );
    assert!(
        !compact(&keyboard[..snapshot]).contains(".lock()"),
        "an earlier keyboard lock now spans PeekMessage"
    );
    assert!(compact(critical).contains("KEY_EVENT_BUILDERS.lock()"));
    assert!(compact(critical)
        .contains("process_message(msg,wparam,lparam,next_key_message,&mutresult)"));
    assert!(keyboard.find("for event in events").unwrap() > critical_start + critical.len());

    let ime = block_after(&code, "let ime_callback = ||");
    let snapshot = ime.find("more_ime_char_coming(").unwrap();
    let ime_critical = block_after(ime, "let text =");
    let critical_start = ime.find(ime_critical).unwrap();
    assert!(
        snapshot < critical_start,
        "IME peek moved inside the window-state lock scope"
    );
    assert!(
        !compact(&ime[..snapshot]).contains(".lock()"),
        "an earlier IME lock now spans PeekMessage"
    );
    assert!(compact(ime_critical).contains("subclass_input.window_state.lock()"));
    assert!(
        compact(ime_critical).contains("process_message(msg,wparam,more_char_coming,&mutresult)")
    );
    assert!(ime.find("if let Some(str) = text").unwrap() > critical_start + ime_critical.len());
    for critical in [critical, ime_critical] {
        let critical = compact(critical);
        for forbidden in [
            "PeekMessageW(",
            "next_key_message_for_keyboard(",
            "more_ime_char_coming(",
            "send_event(",
        ] {
            assert!(
                !critical.contains(forbidden),
                "reentrant operation entered an input lock scope: {forbidden}"
            );
        }
    }
}

#[test]
fn lock_boundary_guard_rejects_reintroducing_each_original_reentrant_peek() {
    let code = code_only(EVENT_LOOP);
    for (marker, snapshot, lock_acquired) in [
        (
            "let keyboard_callback = ||",
            "let next_key_message = next_key_message_for_keyboard(window, msg, wparam);",
            "KEY_EVENT_BUILDERS.lock();",
        ),
        (
            "let ime_callback = ||",
            "let more_char_coming = more_ime_char_coming(window, msg);",
            "subclass_input.window_state.lock();",
        ),
    ] {
        let callback = block_after(&code, marker);
        assert!(callback.contains(snapshot));
        // Mutate an in-memory copy of the production closure to put the peek
        // back inside its critical section. No vendor file is changed.
        assert!(callback.contains(lock_acquired));
        let broken = callback.replacen(snapshot, "", 1).replacen(
            lock_acquired,
            &format!("{lock_acquired} {snapshot}"),
            1,
        );
        let broken_source = code.replacen(callback, &broken, 1);
        assert!(
            std::panic::catch_unwind(|| assert_input_lock_boundaries(&broken_source)).is_err(),
            "source guard accepted an input peek in the locked scope: {marker}"
        );
    }
}

#[test]
fn keyboard_prepeek_covers_press_release_utf16_and_preserves_alt_f4_passthrough() {
    let code = code_only(EVENT_LOOP);
    let helper = block_after(&code, "fn next_key_message_for_keyboard(");
    let policy = compact(block_after(helper, "match msg"));
    let arms: Vec<_> = policy.split(',').filter(|arm| !arm.is_empty()).collect();
    assert_eq!(
        arms.len(),
        3,
        "keyboard prepeek policy requires review after a new route"
    );
    assert_eq!(
        arms[0],
        "win32wm::WM_SYSKEYDOWNifwparam.0==usize::from(VK_F4.0)=>false"
    );
    let (patterns, value) = arms[1].split_once("=>").unwrap();
    assert_eq!(value, "true");
    let actual: std::collections::BTreeSet<_> = patterns.split('|').collect();
    assert_eq!(
        actual,
        [
            "win32wm::WM_KEYDOWN",
            "win32wm::WM_SYSKEYDOWN",
            "win32wm::WM_CHAR",
            "win32wm::WM_SYSCHAR",
            "win32wm::WM_KEYUP",
            "win32wm::WM_SYSKEYUP"
        ]
        .into_iter()
        .collect()
    );
    assert_eq!(arms[2], "_=>false");
    assert!(compact(helper)
        .ends_with("ifneeds_next_key_message{peek_next_key_message(window)}else{None}"));
    let peek = compact(block_after(&code, "fn peek_next_key_message("));
    assert!(peek.contains(
        "PeekMessageW(next_msg.as_mut_ptr(),Some(window),WM_KEYFIRST,WM_KEYLAST,PM_NOREMOVE,)"
    ));
    assert!(
        !peek.contains("PM_REMOVE"),
        "prepeek must not consume a later UTF-16/key message"
    );
}

#[test]
fn ime_prepeek_only_extends_a_commit_for_a_following_char_or_syschar() {
    let code = code_only(EVENT_LOOP);
    let helper = compact(block_after(&code, "fn more_ime_char_coming("));
    assert_eq!(helper,
        "matches!(msg,win32wm::WM_CHAR|win32wm::WM_SYSCHAR)&&matches!(peek_next_key_message(window),Some(next_msg)ifnext_msg.message==WM_CHAR||next_msg.message==WM_SYSCHAR)");
    assert!(!helper.contains(".lock()"));
}

#[test]
fn key_state_reads_happen_before_layout_cache_guards_for_down_up_and_character_finalization() {
    let code = code_only(KEYBOARD);
    for marker in [
        "win32wm::WM_KEYDOWN | win32wm::WM_SYSKEYDOWN =>",
        "win32wm::WM_KEYUP | win32wm::WM_SYSKEYUP =>",
        "if !more_char_coming",
    ] {
        let branch = block_after(&code, marker);
        assert!(
            branch.find("get_kbd_state()").unwrap() < branch.find("LAYOUT_CACHE.lock()").unwrap(),
            "keyboard state read moved inside a layout-cache guard: {marker}"
        );
    }
}
