type PreviewClock = { now(): number; set(callback: () => void, ms: number): ReturnType<typeof setTimeout>; clear(id: ReturnType<typeof setTimeout>): void };
type PreviewHandlers = { dismiss(): void; expire(): void };
export function createPreviewLifetime(clock: PreviewClock, deadline: number, handlers: PreviewHandlers) {
  let hideTimer: ReturnType<typeof setTimeout> | null = null, expiryTimer: ReturnType<typeof setTimeout> | null = null, revision = 0;
  let disposed = false, hovered = false, focused = false, blocked = false;
  let expired = false, expiryGrace = false, hideAt = clock.now() + 3000;
  function cancelHide() {
    ++revision;
    if (hideTimer !== null) clock.clear(hideTimer);
    hideTimer = null;
  }
  function expire() {
    if (!expired && clock.now() >= deadline) {
      expired = true;
      handlers.expire();
    }
  }
  function schedule() {
    cancelHide();
    if (disposed) return;
    expire();
    if (disposed || blocked || focused || (hovered && !expired)) return;
    const at = expired ? (expiryGrace ? hideAt : clock.now()) : Math.min(hideAt, deadline);
    const ticket = revision;
    hideTimer = clock.set(() => {
      if (disposed || ticket !== revision) return;
      expire();
      if (blocked || focused || (hovered && !expired)) { schedule(); return; }
      dispose();
      handlers.dismiss();
    }, Math.max(0, at - clock.now()));
  }
  function hover(value: boolean) {
    if (disposed || hovered === value) return;
    hovered = value;
    if (!value && !expired) hideAt = clock.now() + 1500;
    schedule();
  }
  function focus(value: boolean) {
    if (disposed || focused === value) return;
    focused = value;
    if (!value) { hideAt = clock.now() + 1500; expiryGrace = expired; }
    schedule();
  }
  function block(value: boolean) {
    if (disposed || blocked === value) return;
    blocked = value;
    schedule();
  }
  function dispose() {
    if (disposed) return;
    disposed = true; cancelHide();
    if (expiryTimer !== null) clock.clear(expiryTimer);
    expiryTimer = null;
  }
  expiryTimer = clock.set(() => { if (!disposed) { expire(); schedule(); } }, Math.max(0, deadline - clock.now()));
  schedule();
  return { hover, focus, block, dispose };
}

