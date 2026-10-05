import { useLayoutEffect, useReducer, useRef, useState, type Key, type ReactElement, type RefObject } from "react";
import "./WindowMotion.css";

export const reducedWindowMotion = () => window.matchMedia?.("(prefers-reduced-motion: reduce)").matches ?? false;

/** Keep only the departing UI for a short exit animation, never delay the real
 * navigation/job state. Rapid reopen cancels the old animation and its cleanup. */
export function WindowMotion({ show, children, anchor, travel = false, collapse = false, target, onExited }: {
  show: boolean; children: ReactElement | null;
  anchor?: RefObject<HTMLElement | null>; travel?: boolean; collapse?: boolean; target?: string; onExited?(): void;
}) {
  const [retained, setRetained] = useState(show);
  const wrapper = useRef<HTMLSpanElement>(null);
  const lastChild = useRef(children);
  const lastTravel = useRef(false);
  const departingNode = useRef<HTMLElement | null>(null);
  const exited = useRef(onExited);
  exited.current = onExited;
  if (show && !retained) setRetained(true);
  useLayoutEffect(() => { if (show) lastChild.current = children; });
  useLayoutEffect(() => {
    if (!retained) return;
    const node = (target ? wrapper.current?.querySelector(target) : wrapper.current?.firstElementChild) as HTMLElement | null;
    if (show) {
      if (node && departingNode.current === node && node.matches('[role="dialog"]')) node.focus({ preventScroll: true });
      departingNode.current = null;
    } else departingNode.current = node;
    let cancelled = false;
    const finish = () => { if (!cancelled && !show) { setRetained(false); exited.current?.(); } };
    if (!node?.animate || reducedWindowMotion()) { finish(); return; }
    const dock = anchor?.current;
    const shouldTravel = dock && (show ? lastTravel.current : travel);
    if (!show) lastTravel.current = travel;
    const box = node.getBoundingClientRect();
    // Compose after the existing centering transform. Individual `scale` would
    // also scale translateX(-50%), sending the detail window past its dock.
    const existing = getComputedStyle(node).transform;
    const base = existing === "none" ? "" : existing;
    let folded: Keyframe = { opacity: 0, transform: `${base} translate(0, 10px) scale(.975)` };
    if (shouldTravel && box.width > 0 && box.height > 0) {
      const destination = dock.getBoundingClientRect();
      folded = { opacity: 0, transform: `${base} translate(${destination.left + destination.width / 2 - box.left - box.width / 2}px, ${destination.top + destination.height / 2 - box.top - box.height / 2}px) scale(${destination.width / box.width}, ${destination.height / box.height})` };
    }
    const settled: Keyframe = { opacity: 1, transform: `${base} translate(0, 0) scale(1)` };
    let frames: Keyframe[] = show ? [folded, settled] : [settled, folded];
    if (collapse && !show) frames = [
      { opacity: 1, maxWidth: `${box.width}px`, minWidth: "0", overflow: "hidden" },
      { opacity: 0, maxWidth: "0px", minWidth: "0", paddingLeft: "0px", paddingRight: "0px", marginLeft: "0px", marginRight: "0px", overflow: "hidden" },
    ];
    const animation = node.animate(frames, { duration: shouldTravel ? 230 : collapse ? 150 : 170, easing: "cubic-bezier(.2,.75,.25,1)", fill: "both" });
    animation.finished.then(() => { finish(); animation.cancel(); }, () => undefined);
    return () => { cancelled = true; animation.cancel(); };
  }, [show, retained, target, collapse, anchor, travel]);
  if (!show && !retained) return null;
  return <span ref={wrapper} className="window-motion" inert={!show} aria-hidden={!show || undefined} data-window-exiting={!show || undefined}>{show ? children : lastChild.current}</span>;
}

/** A handful of navigation tabs, not the potentially huge album list. */
export function MovingTabs({ children }: { children: Array<ReactElement | null> }) {
  const previous = useRef<ReactElement[]>([]);
  const finished = useRef(new Set<Key | null>());
  const [, redraw] = useReducer((value: number) => value + 1, 0);
  const current = children.filter((child): child is ReactElement => child !== null);
  const keys = new Set(current.map((child) => child.key));
  keys.forEach((key) => finished.current.delete(key));
  const visible = current.slice();
  previous.current.forEach((child, index) => { if (!keys.has(child.key) && !finished.current.has(child.key)) visible.splice(Math.min(index, visible.length), 0, child); });
  useLayoutEffect(() => {
    previous.current = visible;
    for (const key of finished.current) if (!visible.some((child) => child.key === key)) finished.current.delete(key);
  });
  return visible.map((child) => <WindowMotion key={child.key} show={keys.has(child.key)} collapse onExited={() => {
    finished.current.add(child.key); previous.current = previous.current.filter((item) => item.key !== child.key); redraw();
  }}>{child}</WindowMotion>);
}

export function animateWindowClose(node: HTMLElement | null, done: () => void): () => void {
  if (!node?.animate || reducedWindowMotion()) { done(); return () => {}; }
  let cancelled = false;
  const animation = node.animate([{ opacity: 1, scale: "1" }, { opacity: 0, scale: ".975" }], { duration: 140, easing: "ease-out", fill: "both" });
  animation.finished.then(() => { if (!cancelled) done(); animation.cancel(); }, () => undefined);
  return () => { cancelled = true; animation.cancel(); };
}
