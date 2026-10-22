import { useEffect, useId, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";

/** CHZZK's measured presentation, isolated from its native account and pane state. */
export function SoopConnectionHelp({ children, enabled = true }: { children: ReactNode; enabled?: boolean }) {
  const localId = useId(), id = `soop-help-${localId}`;
  const trigger = useRef<HTMLButtonElement>(null), surface = useRef<HTMLDivElement>(null);
  const [open, setOpen] = useState(false), [position, setPosition] = useState({ left: 12, top: 12 });
  const closeTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const show = () => { clearTimeout(closeTimer.current); if (enabled) setOpen(true); };
  const hide = () => { clearTimeout(closeTimer.current); closeTimer.current = setTimeout(() => setOpen(false), 140); };
  useEffect(() => () => clearTimeout(closeTimer.current), []);
  useEffect(() => { if (!enabled) { clearTimeout(closeTimer.current); setOpen(false); } }, [enabled]);
  useLayoutEffect(() => {
    if (!open) return;
    const measure = () => {
      const button = trigger.current?.getBoundingClientRect(), popup = surface.current?.getBoundingClientRect();
      if (!button || !popup) return;
      const left = Math.max(12, Math.min(button.left, window.innerWidth - popup.width - 12));
      const top = Math.max(12, Math.min(button.bottom + 8, window.innerHeight - popup.height - 12));
      setPosition(previous => previous.left === left && previous.top === top ? previous : { left, top });
    };
    measure();
    const resize = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(measure);
    if (surface.current) resize?.observe(surface.current);
    window.addEventListener("resize", measure); document.addEventListener("scroll", measure, true);
    return () => { resize?.disconnect(); window.removeEventListener("resize", measure); document.removeEventListener("scroll", measure, true); };
  }, [open]);
  return <>
    <button ref={trigger} type="button" className="soop-connection-help" aria-label="SOOP 이용 안내" aria-expanded={open}
      aria-describedby={open ? id : undefined} onMouseEnter={show} onMouseLeave={hide} onFocus={show} onBlur={hide} onClick={show}
      onKeyDown={event => { if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); setOpen(false); } }}>?</button>
    {open && enabled ? createPortal(<div className="soop-connection-help-popover" role="dialog" aria-label="SOOP 이용 안내" aria-modal="false" data-platform-native-occlusion="true"
      style={position} onMouseEnter={show} onMouseLeave={hide}>
      <div ref={surface} id={id} role="tooltip" className="soop-connection-help-content" data-native-dialog-surface="true">{children}</div>
    </div>, document.body) : null}
  </>;
}
