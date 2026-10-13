import { useEffect, useId, useRef, useState, type ReactNode } from "react";
import "./LiveEdgePanel.css";
import { usePanelHandleGesture } from "./usePanelHandleGesture";

/** Hover and player-idle events never resize native cutouts. */
export function LiveEdgePanel({ edge, enabled, label, closeKey, children }: {
  edge: "left" | "top"; enabled: boolean; label: string; closeKey?: unknown; children: ReactNode; runtime?: string;
}) {
  const id = useId(), [open, setOpen] = useState(false);
  const trigger = useRef<HTMLButtonElement>(null);
  const gesture = usePanelHandleGesture(open, edge === "left" ? "x" : "y", setOpen);
  useEffect(() => { setOpen(false); }, [enabled, closeKey]);
  return <div className={`live-edge-panel edge-${edge}${enabled ? " is-enabled" : ""}${open ? " is-open" : ""}`}>
    {enabled ? <button ref={trigger} type="button" className="live-edge-trigger" aria-label={`${label} ${open ? "닫기" : "열기"}`} title={`${label} ${open ? "닫기" : "열기"}`}
      data-native-overlay="true" data-native-preserve-video="true" data-native-interactive-background="true"
      aria-controls={id} aria-expanded={open} {...gesture}
      onKeyDown={event => { if (event.key === "Escape") { event.preventDefault(); setOpen(false); } }}>
      <svg viewBox="0 0 16 24" aria-hidden="true"><path d={open ? "m10 6-5 6 5 6" : "m5 6 5 6-5 6"} /></svg>
    </button> : null}
    <div id={id} className="live-edge-overlay" hidden={enabled && !open} aria-hidden={enabled && !open ? true : undefined}
      role={enabled ? "dialog" : undefined} aria-label={enabled ? label : undefined} aria-modal={enabled ? false : undefined}
      data-native-overlay={enabled && open ? "true" : undefined} data-native-preserve-video={enabled ? "true" : undefined}
      data-native-interactive-background={enabled ? "true" : undefined}
      onKeyDown={event => { if (enabled && event.key === "Escape" && !event.defaultPrevented) { event.preventDefault(); event.stopPropagation(); trigger.current?.focus(); setOpen(false); } }}>
      <div className="live-edge-mask" data-native-dialog-surface={enabled ? "true" : undefined}>
        <div className="live-edge-content">{children}</div>
      </div>
    </div>
  </div>;
}
