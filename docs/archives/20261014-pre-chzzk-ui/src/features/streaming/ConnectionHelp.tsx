import { useEffect, useId, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";

/** A measured, nonmodal hover surface: never adds height to the connection form. */
export function ConnectionHelp({ children }: { children: ReactNode }) {
  const id = useId(), trigger = useRef<HTMLButtonElement>(null), surface = useRef<HTMLDivElement>(null);
  const [open, setOpen] = useState(false), [position, setPosition] = useState({ left: 12, top: 12 });
  const closeTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const show = () => { clearTimeout(closeTimer.current); setOpen(true); };
  const hide = () => { clearTimeout(closeTimer.current); closeTimer.current = setTimeout(() => setOpen(false), 140); };
  useEffect(() => () => clearTimeout(closeTimer.current), []);
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
    <button ref={trigger} type="button" className="connection-help" aria-label="연결 도움말" aria-expanded={open}
      aria-describedby={open ? id : undefined} onMouseEnter={show} onMouseLeave={hide} onFocus={show} onBlur={hide} onClick={show}
      onKeyDown={event => { if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); setOpen(false); } }}>?</button>
    {open ? createPortal(<div className="connection-help-popover" role="dialog" aria-label="연결 도움말" aria-modal="false"
      data-native-overlay="true" data-native-preserve-video="true" data-native-interactive-background="true"
      style={position} onMouseEnter={show} onMouseLeave={hide}>
      <div ref={surface} id={id} role="tooltip" className="connection-help-content" data-native-dialog-surface="true">{children}</div>
    </div>, document.body) : null}
  </>;
}
