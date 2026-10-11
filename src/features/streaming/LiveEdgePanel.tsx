import { useEffect, useId, useRef, useState, type ReactNode } from "react";
import "./LiveEdgePanel.css";

/** Fixed geometry masks only the panel rectangle in native child webviews.
 * Animate the inner surface, never the measured native occlusion rectangle. */
export function LiveEdgePanel({ edge, enabled, label, closeKey, children }: {
  edge: "left" | "top"; enabled: boolean; label: string; closeKey?: unknown; children: ReactNode;
}) {
  const id = useId(), [open, setOpen] = useState(false), [present, setPresent] = useState(false);
  const panel = useRef<HTMLDivElement>(null), trigger = useRef<HTMLButtonElement>(null);
  const closeTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const hideTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const reveal = () => { clearTimeout(closeTimer.current); clearTimeout(hideTimer.current); setPresent(true); setOpen(true); };
  const hide = () => {
    clearTimeout(closeTimer.current); clearTimeout(hideTimer.current); setOpen(false);
    hideTimer.current = setTimeout(() => setPresent(false), 180);
  };
  const deferHide = () => {
    clearTimeout(closeTimer.current);
    closeTimer.current = setTimeout(() => {
      // Clicking a toolbar button must not pin a hover panel forever. Only
      // keyboard navigation keeps a focus-visible control exposed.
      if (!panel.current?.querySelector(":focus-visible")) hide();
    }, 320);
  };
  useEffect(() => {
    clearTimeout(closeTimer.current); clearTimeout(hideTimer.current); setOpen(false); setPresent(false);
  }, [enabled, closeKey]);
  useEffect(() => () => { clearTimeout(closeTimer.current); clearTimeout(hideTimer.current); }, []);
  return <div className={`live-edge-panel edge-${edge}${enabled ? " is-enabled" : ""}${open ? " is-open" : ""}`}>
    {enabled ? <button ref={trigger} type="button" className="live-edge-trigger" aria-label={`${label} 열기`} title={label}
      aria-controls={id} aria-expanded={open} onPointerEnter={reveal} onPointerLeave={deferHide} onFocus={reveal}
      onClick={reveal} onKeyDown={event => { if (event.key === "Escape") { event.preventDefault(); hide(); } }} /> : null}
    <div ref={panel} id={id} className="live-edge-overlay" hidden={enabled && !present} aria-hidden={enabled && !present ? true : undefined}
      role={enabled ? "dialog" : undefined} aria-label={enabled ? label : undefined} aria-modal={enabled ? false : undefined}
      data-native-overlay={enabled && present ? "true" : undefined} data-native-preserve-video={enabled ? "true" : undefined}
      data-native-interactive-background={enabled ? "true" : undefined}
      onPointerEnter={enabled ? reveal : undefined} onPointerLeave={enabled ? deferHide : undefined}
      onFocus={enabled ? reveal : undefined} onBlur={enabled ? event => { if (!event.currentTarget.contains(event.relatedTarget as Node | null)) deferHide(); } : undefined}
      onKeyDown={event => { if (enabled && event.key === "Escape" && !event.defaultPrevented) { event.preventDefault(); event.stopPropagation(); trigger.current?.focus(); hide(); } }}>
      <div className="live-edge-mask" data-native-dialog-surface={enabled ? "true" : undefined}>
        <div className="live-edge-content">{children}</div>
      </div>
    </div>
  </div>;
}
