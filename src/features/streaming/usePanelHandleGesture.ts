import { useRef, type MouseEvent, type PointerEvent } from "react";

/** Commit a deliberate pull once; never resize native video on every pointer move. */
export function usePanelHandleGesture(open: boolean, axis: "x" | "y", onChange: (open: boolean) => void) {
  const pull = useRef<{ id: number; start: number; open: boolean; committed: boolean } | null>(null);
  const suppressClick = useRef(false);
  const coordinate = (event: PointerEvent<HTMLButtonElement>) => axis === "x" ? event.clientX : event.clientY;
  const finish = (event: PointerEvent<HTMLButtonElement>) => {
    if (pull.current?.id !== event.pointerId) return;
    pull.current = null;
    if (event.currentTarget.hasPointerCapture?.(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId);
  };
  return {
    onPointerDown(event: PointerEvent<HTMLButtonElement>) {
      if (event.button !== 0 || event.isPrimary === false) return;
      suppressClick.current = false;
      pull.current = { id: event.pointerId, start: coordinate(event), open, committed: false };
      event.currentTarget.setPointerCapture?.(event.pointerId);
    },
    onPointerMove(event: PointerEvent<HTMLButtonElement>) {
      const gesture = pull.current;
      if (!gesture || gesture.id !== event.pointerId || gesture.committed) return;
      const distance = (coordinate(event) - gesture.start) * (axis === "x" ? 1 : -1) * (gesture.open ? -1 : 1);
      if (distance < 24) return;
      event.preventDefault(); gesture.committed = true; suppressClick.current = true;
      event.currentTarget.focus({ preventScroll: true }); onChange(!gesture.open);
    },
    onPointerUp: finish,
    onPointerCancel(event: PointerEvent<HTMLButtonElement>) { suppressClick.current = true; finish(event); },
    onLostPointerCapture(event: PointerEvent<HTMLButtonElement>) { if (pull.current?.id === event.pointerId) pull.current = null; },
    onClick(event: MouseEvent<HTMLButtonElement>) {
      // Pointer capture also delivers a click after a drag. Do not undo the pull.
      if (suppressClick.current && event.detail !== 0) { suppressClick.current = false; event.preventDefault(); return; }
      suppressClick.current = false; onChange(!open);
    },
  };
}
