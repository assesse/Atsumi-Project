import { cloneElement, useEffect, useRef, useState, type CSSProperties, type ReactElement, type ReactNode } from "react";

type Split = { columns: number; rows: number; strip: number[] };
type Axis = "columns" | "rows" | `strip-${number}`;
const clamp = (value: number) => Math.max(20, Math.min(80, Math.round(value)));
const read = (key: string): Split => {
  try {
    const value = JSON.parse(localStorage.getItem(key) ?? "null");
    if (Number.isFinite(value?.columns) && Number.isFinite(value?.rows)) return { columns: clamp(value.columns), rows: clamp(value.rows), strip: Array.isArray(value.strip) && value.strip.length === 4 && value.strip.every((weight: unknown) => typeof weight === "number" && Number.isFinite(weight) && weight >= 5 && weight <= 100) ? value.strip : [25, 25, 25, 25] };
  } catch { /* Optional layout only. Never stores channels or account state. */ }
  return { columns: 50, rows: 50, strip: [25, 25, 25, 25] };
};

/** Resize presentation cells without replacing their native receiver or chat. */
export function LivePaneGrid({ kind, className, children, onResizing, storageId = kind, label: suppliedLabel, accessory }: {
  kind: "video" | "chat" | "pair" | "channel"; className: string;
  children: ReactElement<{ style?: CSSProperties }>[];
  onResizing(value: boolean): void;
  storageId?: string; label?: string; accessory?: ReactNode;
}) {
  const key = `atsumi.live.panes.${storageId}.v1`;
  const [split, setSplit] = useState(() => read(key));
  const [dragging, setDragging] = useState(false);
  const count = children.length, label = suppliedLabel ?? (kind === "video" ? "방송" : kind === "pair" ? "방송 묶음" : "채팅");
  const isColumns = (axis: Axis) => axis !== "rows";
  const valueFor = (current: Split, axis: Axis) => {
    if (axis === "columns" || axis === "rows") return current[axis];
    const index = Number(axis.slice(6));
    return Math.round(current.strip.slice(0, index + 1).reduce((a, b) => a + b, 0) / current.strip.slice(0, count).reduce((a, b) => a + b, 0) * 100);
  };
  const update = (current: Split, axis: Axis, value: number): Split => {
    if (axis === "columns" || axis === "rows") return { ...current, [axis]: clamp(value) };
    const index = Number(axis.slice(6)), weights = [...current.strip];
    const total = weights.slice(0, count).reduce((a, b) => a + b, 0), before = weights.slice(0, index).reduce((a, b) => a + b, 0);
    const pair = weights[index]! + weights[index + 1]!;
    weights[index] = Math.max(5, Math.min(pair - 5, value * total / 100 - before));
    weights[index + 1] = pair - weights[index]!;
    return { ...current, strip: weights };
  };
  const grid = useRef<HTMLDivElement>(null);
  const drag = useRef<{ id: number; axis: Axis; bounds: DOMRect } | null>(null);
  const frame = useRef(0), next = useRef<{ axis: Axis; value: number } | null>(null);
  const notify = useRef(onResizing); notify.current = onResizing;
  const flush = () => {
    cancelAnimationFrame(frame.current); frame.current = 0;
    if (next.current) { const value = next.current; next.current = null; setSplit(current => update(current, value.axis, value.value)); }
  };
  const finish = () => { flush(); drag.current = null; setDragging(false); notify.current(false); };
  const finishRef = useRef(finish); finishRef.current = finish;
  useEffect(() => {
    const timer = setTimeout(() => { try { localStorage.setItem(key, JSON.stringify(split)); } catch { /* Optional preference. */ } }, 200);
    return () => clearTimeout(timer);
  }, [key, split]);
  useEffect(() => {
    const blur = () => { if (drag.current) finishRef.current(); };
    const hidden = () => { if (document.hidden) blur(); };
    window.addEventListener("blur", blur); document.addEventListener("visibilitychange", hidden);
    return () => {
      window.removeEventListener("blur", blur); document.removeEventListener("visibilitychange", hidden);
      cancelAnimationFrame(frame.current);
      if (drag.current) notify.current(false);
    };
  }, []);
  const divider = (axis: Axis) => <div key={axis} className={`mado-divider live-pane-divider split-${axis}${axis.startsWith('strip-') ? ' split-strip' : ''}`} role="separator" tabIndex={0}
    style={axis.startsWith('strip-') ? { gridColumn: Number(axis.slice(6)) * 2 + 2, gridRow: 1 } : undefined}
    aria-label={`${label} ${axis.startsWith('strip-') ? `${Number(axis.slice(6)) + 1}·${Number(axis.slice(6)) + 2}` : axis === "columns" ? "좌우" : "상하"} 크기`} aria-orientation={isColumns(axis) ? "vertical" : "horizontal"}
    aria-valuemin={axis.startsWith('strip-') ? 0 : 20} aria-valuemax={axis.startsWith('strip-') ? 100 : 80} aria-valuenow={valueFor(split, axis)} title="끌거나 방향키로 크기 조절 · 두 번 클릭해 균등 배치"
    onDoubleClick={() => setSplit(current => axis.startsWith('strip-') ? { ...current, strip: [25, 25, 25, 25] } : { ...current, [axis]: 50 })}
    onKeyDown={event => {
      const lower = isColumns(axis) ? "ArrowLeft" : "ArrowUp", upper = isColumns(axis) ? "ArrowRight" : "ArrowDown";
      if (![lower, upper, "Home", "End"].includes(event.key)) return;
      event.preventDefault(); event.stopPropagation();
      setSplit(current => update(current, axis, event.key === "Home" ? 0 : event.key === "End" ? 100 : valueFor(current, axis) + (event.key === lower ? -1 : 1) * (event.shiftKey ? 10 : 2)));
    }}
    onPointerDown={event => {
      if (event.button !== 0 || !grid.current) return;
      event.preventDefault(); drag.current = { id: event.pointerId, axis, bounds: grid.current.getBoundingClientRect() };
      event.currentTarget.setPointerCapture?.(event.pointerId); setDragging(true); notify.current(true);
    }}
    onPointerMove={event => {
      const current = drag.current;
      if (!current || current.id !== event.pointerId) return;
      const size = isColumns(current.axis) ? current.bounds.width : current.bounds.height;
      const offset = isColumns(current.axis) ? event.clientX - current.bounds.left : event.clientY - current.bounds.top;
      const dividerSize = (current.axis.startsWith('strip-') ? count - 1 : 1) * 10;
      if (size <= dividerSize) return;
      const stripIndex = current.axis.startsWith('strip-') ? Number(current.axis.slice(6)) : 0;
      next.current = { axis: current.axis, value: (offset - stripIndex * 10 - 5) / (size - dividerSize) * 100 };
      if (!frame.current) frame.current = requestAnimationFrame(flush);
    }} onPointerUp={finish} onPointerCancel={finish} onLostPointerCapture={() => { if (drag.current) finish(); }} />;
  return <div ref={grid} className={`${className} live-pane-grid${dragging ? " is-resizing" : ""}`} data-pane-count={count}
    style={{ "--pane-first-column": `${split.columns}fr`, "--pane-second-column": `${100 - split.columns}fr`, "--pane-first-row": `${split.rows}fr`, "--pane-second-row": `${100 - split.rows}fr`, ...Object.fromEntries(split.strip.map((weight, index) => [`--pane-strip-${index}`, `${weight}fr`])) } as CSSProperties}>
    {accessory}
    {children.map((child, index) => cloneElement(child, { style: { ...child.props.style, gridColumn: index % 2 ? 3 : 1, gridRow: index < 2 ? 1 : 3 } }))}
    {count > 1 ? divider("columns") : null}{count > 2 ? divider("rows") : null}
    {kind === 'video' && count > 2 ? Array.from({ length: count - 1 }, (_, index) => divider(`strip-${index}`)) : null}
  </div>;
}
