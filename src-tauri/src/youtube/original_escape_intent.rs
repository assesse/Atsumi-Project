//! Escape-only native intent for an asynchronous original-player exit message.
//! No general key capture, focus movement, DOM inspection or delayed key grace.
use crate::{platform_native_geometry::InputGate, platform_window_fullscreen::Grant};
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

const MAX_AGE: Duration = Duration::from_millis(500);
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Binding {
    pub epoch: u64,
    pub grant: Grant,
}
struct Receipt {
    binding: Binding,
    created: Instant,
}
#[derive(Default)]
pub(super) struct Intent {
    pending: Mutex<Option<Receipt>>,
}
impl Intent {
    pub(super) fn clear(&self) {
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
    }
    fn observe(&self, binding: Binding, now: Instant) {
        *self.pending.lock().unwrap_or_else(|e| e.into_inner()) = Some(Receipt {
            binding,
            created: now,
        });
    }
    fn take(&self, binding: &Binding, now: Instant) -> bool {
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
            .is_some_and(|receipt| {
                receipt.binding == *binding
                    && now
                        .checked_duration_since(receipt.created)
                        .is_some_and(|age| age <= MAX_AGE)
            })
    }
    /// Called only for a canonical/current Escape + enabled=false message.
    /// The exact returned grant must be released, never the later current one.
    pub(super) fn exit(
        &self,
        binding: Option<Binding>,
        gate: InputGate,
        context: impl FnOnce() -> InputGate,
        now: Instant,
    ) -> (InputGate, Option<Grant>, bool) {
        let Some(binding) = binding else {
            self.clear();
            return (InputGate::InvalidInput, None, false);
        };
        let receipt = self.take(&binding, now);
        // Even a currently held key needs its exact native event. Otherwise an
        // old press for grant A could close a newly acquired grant B, or a
        // consumed/lost-focus event could be replayed while the key stays down.
        if !receipt {
            return (
                if gate.allowed() {
                    InputGate::InvalidInput
                } else {
                    gate
                },
                None,
                false,
            );
        }
        if gate.allowed() {
            return (gate, Some(binding.grant), false);
        }
        if gate == InputGate::KeyReleased && receipt {
            let context = context();
            return (
                context,
                context.allowed().then_some(binding.grant),
                context.allowed(),
            );
        }
        (gate, None, false)
    }
}
fn eligible(key: u32, down: bool, repeated: bool, released: bool, modified: bool) -> bool {
    key == 0x1B && down && !repeated && !released && !modified
}

/// Escape is a native accelerator even without modifiers. Leave it unhandled
/// so the original document still decides editable/IME/menu eligibility.
#[cfg(windows)]
pub(super) fn install(
    controller: &webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Controller,
    root: usize,
    pane: usize,
    intent: Arc<Intent>,
    current: Arc<dyn Fn() -> Option<Binding> + Send + Sync>,
) -> windows::core::Result<()> {
    use webview2_com::{
        AcceleratorKeyPressedEventHandler, FocusChangedEventHandler,
        Microsoft::Web::WebView2::Win32::{
            COREWEBVIEW2_KEY_EVENT_KIND, COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN,
            COREWEBVIEW2_PHYSICAL_KEY_STATUS,
        },
    };
    use windows::Win32::UI::Input::KeyboardAndMouse::GetKeyState;
    let lost = intent.clone();
    unsafe {
        let mut focus_token = 0;
        controller.add_LostFocus(
            &FocusChangedEventHandler::create(Box::new(move |_, _| {
                lost.clear();
                Ok(())
            })),
            &mut focus_token,
        )?;
        let mut key_token = 0;
        controller.add_AcceleratorKeyPressed(
            &AcceleratorKeyPressedEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else { return Ok(()) };
                let mut key = 0;
                args.VirtualKey(&mut key)?;
                if key != 0x1B {
                    return Ok(());
                }
                let mut kind = COREWEBVIEW2_KEY_EVENT_KIND::default();
                args.KeyEventKind(&mut kind)?;
                // Matching keyup must not erase the receipt before async delivery.
                if kind != COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN {
                    return Ok(());
                }
                let mut physical = COREWEBVIEW2_PHYSICAL_KEY_STATUS::default();
                args.PhysicalKeyStatus(&mut physical)?;
                let modified = physical.IsMenuKeyDown.as_bool()
                    || [0x10, 0x11, 0x12, 0x5B, 0x5C]
                        .into_iter()
                        .any(|key| GetKeyState(key) as u16 & 0x8000 != 0);
                if !eligible(
                    key,
                    true,
                    physical.WasKeyDown.as_bool() || physical.RepeatCount != 1,
                    physical.IsKeyReleased.as_bool(),
                    modified,
                ) {
                    intent.clear();
                    return Ok(());
                }
                let Some(binding) = current() else {
                    intent.clear();
                    return Ok(());
                };
                if binding.epoch == 0
                    || !crate::platform_native_geometry::trusted_escape_receipt_context(root, pane)
                        .allowed()
                {
                    intent.clear();
                    return Ok(());
                }
                intent.observe(binding, Instant::now());
                // No Handled call, script evaluation, window mutation or wait.
                Ok(())
            })),
            &mut key_token,
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn binding() -> Binding {
        Binding {
            epoch: 2,
            grant: Grant {
                owner: "test-owner".into(),
                document_epoch: 2,
                revision: 8,
                window_key: "main-incarnation".into(),
            },
        }
    }
    #[test]
    fn only_plain_first_escape_keydown_is_eligible() {
        assert!(eligible(0x1B, true, false, false, false));
        for key in [1, 0x46, 0x53, 13, 32, 0xE5] {
            assert!(!eligible(key, true, false, false, false));
        }
        for flags in [
            (false, false, false, false),
            (true, true, false, false),
            (true, false, true, false),
            (true, false, false, true),
        ] {
            assert!(!eligible(0x1B, flags.0, flags.1, flags.2, flags.3));
        }
    }
    #[test]
    fn receipt_accepts_one_released_escape_and_only_within_deadline() {
        let intent = Intent::default();
        let now = Instant::now();
        let binding = binding();
        intent.observe(binding.clone(), now);
        assert_eq!(
            intent.exit(
                Some(binding.clone()),
                InputGate::KeyReleased,
                || InputGate::Accepted,
                now + MAX_AGE
            ),
            (InputGate::Accepted, Some(binding.grant.clone()), true)
        );
        assert_eq!(
            intent.exit(
                Some(binding.clone()),
                InputGate::KeyReleased,
                || panic!("no receipt"),
                now
            ),
            (InputGate::KeyReleased, None, false)
        );
        intent.observe(binding.clone(), now);
        assert!(!intent.take(&binding, now + MAX_AGE + Duration::from_nanos(1)));
        intent.observe(binding.clone(), now + Duration::from_secs(1));
        assert!(!intent.take(&binding, now));
    }
    #[test]
    fn wrong_epoch_owner_revision_and_window_never_match_and_consume_attempt() {
        let intent = Intent::default();
        let now = Instant::now();
        let original = binding();
        let mut changed = Vec::new();
        let mut item = original.clone();
        item.epoch += 1;
        changed.push(item);
        let mut item = original.clone();
        item.grant.owner = "other-owner".into();
        changed.push(item);
        let mut item = original.clone();
        item.grant.revision += 1;
        changed.push(item);
        let mut item = original.clone();
        item.grant.window_key = "new-window".into();
        changed.push(item);
        let mut item = original.clone();
        item.grant.document_epoch += 1;
        changed.push(item);
        for other in changed {
            intent.observe(original.clone(), now);
            assert!(!intent.take(&other, now));
            assert!(!intent.take(&original, now));
        }
    }
    #[test]
    fn lost_focus_and_independent_controllers_cannot_reuse_receipts() {
        let one = Intent::default();
        let two = Intent::default();
        let now = Instant::now();
        let binding = binding();
        one.observe(binding.clone(), now);
        assert!(!two.take(&binding, now));
        one.clear();
        assert!(!one.take(&binding, now));
        one.observe(binding.clone(), now);
        assert_eq!(
            one.exit(None, InputGate::KeyReleased, || panic!("closed"), now),
            (InputGate::InvalidInput, None, false)
        );
        assert!(!one.take(&binding, now));
    }
    #[test]
    fn receipt_cannot_override_focus_visibility_or_another_gate_denial() {
        let intent = Intent::default();
        let now = Instant::now();
        let binding = binding();
        for denied in [
            InputGate::InvalidInput,
            InputGate::WindowUnavailable,
            InputGate::FocusMismatch,
            InputGate::PointerOutside,
        ] {
            intent.observe(binding.clone(), now);
            assert_eq!(
                intent.exit(
                    Some(binding.clone()),
                    denied,
                    || panic!("not a key release"),
                    now
                ),
                (denied, None, false)
            );
            intent.observe(binding.clone(), now);
            assert_eq!(
                intent.exit(
                    Some(binding.clone()),
                    InputGate::KeyReleased,
                    || denied,
                    now
                ),
                (denied, None, false)
            );
        }
    }
    #[test]
    fn held_key_uses_existing_permission_and_discards_duplicate_receipt() {
        let intent = Intent::default();
        let now = Instant::now();
        let binding = binding();
        intent.observe(binding.clone(), now);
        assert_eq!(
            intent.exit(
                Some(binding.clone()),
                InputGate::Accepted,
                || panic!("existing physical permission"),
                now
            ),
            (InputGate::Accepted, Some(binding.grant.clone()), false)
        );
        assert!(!intent.take(&binding, now));
    }
    #[test]
    fn a_held_old_press_never_releases_a_new_grant_or_replays_after_focus_loss() {
        let intent = Intent::default();
        let now = Instant::now();
        let old = binding();
        let mut new = old.clone();
        new.grant.revision += 1;
        intent.observe(old.clone(), now);
        assert_eq!(
            intent.exit(
                Some(new.clone()),
                InputGate::Accepted,
                || panic!("foreign receipt"),
                now
            ),
            (InputGate::InvalidInput, None, false)
        );
        intent.observe(new.clone(), now);
        intent.clear();
        assert_eq!(
            intent.exit(
                Some(new.clone()),
                InputGate::Accepted,
                || panic!("lost focus"),
                now
            ),
            (InputGate::InvalidInput, None, false)
        );
        intent.observe(new.clone(), now);
        assert_eq!(
            intent
                .exit(
                    Some(new.clone()),
                    InputGate::Accepted,
                    || panic!("held"),
                    now
                )
                .1,
            Some(new.grant.clone())
        );
        assert_eq!(
            intent.exit(
                Some(new.clone()),
                InputGate::Accepted,
                || panic!("consumed"),
                now
            ),
            (InputGate::InvalidInput, None, false)
        );
        intent.observe(new.clone(), now);
        assert_eq!(
            intent.exit(
                Some(new),
                InputGate::Accepted,
                || panic!("expired"),
                now + MAX_AGE + Duration::from_nanos(1)
            ),
            (InputGate::InvalidInput, None, false)
        );
    }
}
