import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { hitomiTourSteps, type TourStep } from "../tutorial/tourSteps";
import { tutorialActionEvent, tutorialStepEvent, tutorialFinishedEvent, type TutorialActionResult } from "../tutorial/tourActions";
import { spotlightRect, tourPlacement, type TourRect } from "../tutorial/tourGeometry";
import "./TutorialDialog.css";

type Props = { open: boolean; onClose: () => void; onStart?: () => void; onStepChange?: (id: string) => void; steps?: readonly TourStep[] };
let visitSequence = 0;
const focusableSelector = 'button:not(:disabled), input:not(:disabled), select:not(:disabled), textarea:not(:disabled), a[href], [tabindex="0"]';
const visible = (node: HTMLElement) => !node.closest('[hidden], [inert], [aria-hidden="true"]') && node.getClientRects().length > 0 && getComputedStyle(node).visibility !== "hidden";

/** A non-modal top-layer popover leaves the real spotlight target clickable. */
export function TutorialDialog({ open, onClose, onStart, onStepChange, steps = hitomiTourSteps }: Props) {
  const [index, setIndex] = useState(0);
  const [satisfied, setSatisfied] = useState(false);
  const [emptyAction, setEmptyAction] = useState(false);
  const [actionState, setActionState] = useState<{ phase: "idle" | "pending" | "error"; message?: string }>({ phase: "idle" });
  const actionStarted = useRef(false), pending = useRef(false);
  const [targetRect, setTargetRect] = useState<TourRect | null>(null);
  const [viewport, setViewport] = useState({ width: window.innerWidth, height: window.innerHeight });
  const [size, setSize] = useState({ width: 360, height: 300 });
  const [modalHost, setModalHost] = useState<HTMLElement | null>(null);
  const layer = useRef<HTMLDivElement>(null), bubble = useRef<HTMLElement>(null);
  const target = useRef<HTMLElement | null>(null), opener = useRef<HTMLElement | null>(null);
  const callbacks = useRef({ onClose, onStart, onStepChange });
  callbacks.current = { onClose, onStart, onStepChange };
  const pendingAdvance = useRef<number | undefined>(undefined);
  const step = steps[index];
  const close = () => callbacks.current.onClose();
  const advance = () => {
    if (index === steps.length - 1) {
      window.dispatchEvent(new CustomEvent(tutorialFinishedEvent, { detail: step?.id }));
      callbacks.current.onClose();
    }
    else setIndex(index + 1);
  };
  const next = () => { if ((!step?.action || satisfied) && !pending.current) advance(); };
  const actions = useRef({ close, advance });
  actions.current = { close, advance };

  useLayoutEffect(() => {
    if (!open) {
      // Reset while closed so replay never prepares the previous step's workspace.
      setIndex(0);
      setTargetRect(null);
      setModalHost(null);
      return;
    }
    opener.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    setIndex(0);
    document.documentElement.dataset.tutorialOpen = "true";
    callbacks.current.onStart?.();
    return () => {
      delete document.documentElement.dataset.tutorialOpen;
      delete document.documentElement.dataset.tutorialStep;
      delete document.documentElement.dataset.tutorialVisit;
      window.clearTimeout(pendingAdvance.current);
      if (opener.current?.isConnected) opener.current.focus({ preventScroll: true });
    };
  }, [open]);

  useLayoutEffect(() => {
    if (!open || !step) return;
    document.documentElement.dataset.tutorialStep = step.id;
    document.documentElement.dataset.tutorialVisit = String(++visitSequence);
    actionStarted.current = false; pending.current = false;
    setActionState({ phase: "idle" }); setSatisfied(false); setEmptyAction(false);
    callbacks.current.onStepChange?.(step.id);
    window.dispatchEvent(new CustomEvent(tutorialStepEvent, { detail: step.id }));
  }, [open, step]);

  useLayoutEffect(() => {
    if (!open) return;
    let frame = 0, disposed = false;
    let observed: HTMLElement | null = null;
    const bringToFront = () => {
      const node = layer.current;
      if (!node?.showPopover) return;
      try { node.hidePopover(); node.showPopover(); } catch { /* Fixed-position fallback. */ }
    };
    const measure = () => {
      frame = 0;
      if (disposed) return;
      const found = step ? [...document.querySelectorAll<HTMLElement>(step.target)].find(visible) ?? null : null;
      if (found !== observed) {
        if (observed) resize?.unobserve(observed);
        observed = found;
        if (found) {
          const rect = found.getBoundingClientRect();
          if (rect.top < 0 || rect.bottom > window.innerHeight || rect.left < 0 || rect.right > window.innerWidth) found.scrollIntoView?.({ block: "nearest", inline: "nearest", behavior: "instant" });
          resize?.observe(found);
        }
        bringToFront();
      }
      target.current = found;
      const registered = step?.satisfiedSelector && found
        ? (step.satisfiedSelector.startsWith(":scope") ? found.matches(step.satisfiedSelector.replace(":scope", "")) : !!found.querySelector(step.satisfiedSelector))
        : false;
      const available = step?.actionSelector && found ? found.querySelector(step.actionSelector) : null;
      setSatisfied(Boolean(registered && (!step?.actionSelector || !available)));
      setEmptyAction(Boolean(found && step?.emptyActionFallback && step.actionSelector && !available && !registered));
      // A popover outside a modal dialog remains inert even when visually on top.
      // Move only our layer into that dialog, without changing its lifecycle.
      const modal = found?.closest<HTMLDialogElement>("dialog[open]") ?? document.querySelector<HTMLDialogElement>("dialog[open]");
      setModalHost(old => old === modal ? old : modal);
      const view = { width: window.innerWidth, height: window.innerHeight };
      setViewport(old => old.width === view.width && old.height === view.height ? old : view);
      const parts = found && step?.spotlightSelector ? [...found.querySelectorAll<HTMLElement>(step.spotlightSelector)].filter(visible).map(node => node.getBoundingClientRect()) : [];
      const bounds = parts.length ? {
        left: Math.min(...parts.map(rect => rect.left)), top: Math.min(...parts.map(rect => rect.top)),
        width: Math.max(...parts.map(rect => rect.right)) - Math.min(...parts.map(rect => rect.left)),
        height: Math.max(...parts.map(rect => rect.bottom)) - Math.min(...parts.map(rect => rect.top)),
      } : found?.getBoundingClientRect();
      const rect = bounds ? spotlightRect(bounds, view.width, view.height) : null;
      setTargetRect(old => JSON.stringify(old) === JSON.stringify(rect) ? old : rect);
      const actual = bubble.current?.getBoundingClientRect();
      if (actual?.height) setSize(old => old.width === actual.width && old.height === actual.height ? old : { width: actual.width, height: actual.height });
    };
    const schedule = () => { if (!frame) frame = requestAnimationFrame(measure); };
    const resize = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(schedule);
    if (bubble.current) resize?.observe(bubble.current);
    const mutation = new MutationObserver(records => {
      if (records.some(record => !layer.current?.contains(record.target))) schedule();
    });
    mutation.observe(document.body, { subtree: true, childList: true, attributes: true, attributeFilter: ["open", "hidden", "class", "style", "aria-hidden", "data-tour", "data-favorite", "data-running", "disabled"] });
    window.addEventListener("resize", schedule); window.addEventListener("scroll", schedule, true);
    bringToFront(); schedule();
    const focusFrame = requestAnimationFrame(() => bubble.current?.querySelector<HTMLButtonElement>("button:not(:disabled)")?.focus({ preventScroll: true }));
    return () => {
      disposed = true; cancelAnimationFrame(frame); cancelAnimationFrame(focusFrame);
      mutation.disconnect(); resize?.disconnect();
      window.removeEventListener("resize", schedule); window.removeEventListener("scroll", schedule, true);
      target.current = null;
    };
  }, [open, index, step, modalHost]);

  useEffect(() => {
    if (!open) return;
    const actionTarget = (node: EventTarget | null): HTMLElement | null => {
      if (!(node instanceof Element) || !step?.action || !target.current?.contains(node)) return null;
      const control = step.actionSelector ? node.closest<HTMLElement>(step.actionSelector) : target.current;
      if (!control || !target.current.contains(control) || control.matches(':disabled, [aria-disabled="true"]') || !visible(control)) return null;
      if (step.satisfiedSelector?.startsWith(":scope") && control.matches(step.satisfiedSelector.replace(":scope", ""))) return null;
      // A card also contains download/bookmark/tag controls. Only its body can open a detail.
      if (step.action === "dblclick" && node.closest("button, a, input, select, textarea")) return null;
      return control;
    };
    const stop = (event: Event) => { event.preventDefault(); event.stopImmediatePropagation(); };
    const pointer = (event: Event) => {
      if (bubble.current?.contains(event.target as Node)) return;
      if (!actionTarget(event.target) || pending.current) { stop(event); return; }
      if (event instanceof MouseEvent && (event.ctrlKey || event.metaKey || event.altKey
        || (step?.action === "contextmenu" ? ["click", "dblclick"].includes(event.type) || event.button !== 2 : event.button !== 0 || event.type === "contextmenu"))) { stop(event); return; }
      if (event.type === step?.action) {
        actionStarted.current = true;
        if (step.waitForResult) {
          pending.current = true;
          setActionState({ phase: "pending" });
        } else {
          window.clearTimeout(pendingAdvance.current);
          pendingAdvance.current = window.setTimeout(() => actions.current.advance(), 0);
        }
      }
    };
    const keys = (event: KeyboardEvent) => {
      if (event.key === "Escape") { stop(event); actions.current.close(); return; }
      if (event.key === "Tab") {
        stop(event);
        const targets = step?.action && target.current && !pending.current
          ? (step.actionSelector ? [...target.current.querySelectorAll<HTMLElement>(step.actionSelector)] : [target.current]).filter(node => !node.matches(':disabled, [aria-disabled="true"]') && visible(node)) : [];
        const controls = [...(bubble.current?.querySelectorAll<HTMLElement>(focusableSelector) ?? [])].filter(visible);
        const cycle = [...targets, ...controls];
        const at = cycle.indexOf(document.activeElement as HTMLElement);
        cycle[(at + (event.shiftKey ? -1 : 1) + cycle.length) % cycle.length]?.focus({ preventScroll: true });
        return;
      }
      const control = actionTarget(event.target);
      if (!pending.current && control && step?.action === "dblclick" && event.key === "Enter") {
        stop(event); control.dispatchEvent(new MouseEvent("dblclick", { bubbles: true })); return;
      }
      if (!pending.current && control && step?.action === "contextmenu" && (event.key === "ContextMenu" || (event.shiftKey && event.key === "F10"))) {
        stop(event); control.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, button: 2 })); return;
      }
      if (bubble.current?.contains(event.target as Node)) return;
      if (!pending.current && control && step?.action === "click" && ["Enter", " "].includes(event.key) && event.target instanceof HTMLButtonElement) return;
      stop(event);
    };
    const focus = (event: FocusEvent) => {
      if (!bubble.current?.contains(event.target as Node) && !actionTarget(event.target)) bubble.current?.querySelector<HTMLButtonElement>("button:not(:disabled)")?.focus({ preventScroll: true });
    };
    const wheel = (event: Event) => { if (!bubble.current?.contains(event.target as Node) && !(step?.actionSelector && target.current?.contains(event.target as Node))) stop(event); };
    const result = (event: Event) => {
      const detail = (event as CustomEvent<TutorialActionResult>).detail;
      if (!step?.waitForResult || !actionStarted.current || detail?.stepId !== step.id || detail.visit !== document.documentElement.dataset.tutorialVisit) return;
      if (detail.phase === "success") { pending.current = false; actions.current.advance(); }
      else { pending.current = detail.phase === "pending"; setActionState({ phase: detail.phase, message: detail.message }); }
    };
    const events = ["pointerdown", "mousedown", "click", "dblclick", "contextmenu"];
    events.forEach(name => window.addEventListener(name, pointer, true));
    window.addEventListener("keydown", keys, true); window.addEventListener("focusin", focus, true);
    window.addEventListener("wheel", wheel, { capture: true, passive: false });
    window.addEventListener(tutorialActionEvent, result);
    return () => {
      events.forEach(name => window.removeEventListener(name, pointer, true));
      window.removeEventListener("keydown", keys, true); window.removeEventListener("focusin", focus, true); window.removeEventListener("wheel", wheel, true);
      window.removeEventListener(tutorialActionEvent, result);
      window.clearTimeout(pendingAdvance.current);
    };
  }, [open, step]);

  if (!open) return null;
  const { bubble: position, line } = tourPlacement(targetRect, viewport, size);
  const hole = targetRect;
  const shade = `M0 0H${viewport.width}V${viewport.height}H0Z${hole ? ` M${hole.left} ${hole.top}v${hole.height}h${hole.width}v-${hole.height}Z` : ""}`;
  const content = <div ref={layer} popover="manual" className="tutorial-tour" role="dialog" aria-modal="true" aria-labelledby="tutorial-title" aria-describedby="tutorial-description" data-tutorial-active="true" data-tour-step={step?.id} data-native-overlay="true" data-native-preserve-video="true">
    <svg className="tutorial-shade" data-native-dialog-surface="true" width={viewport.width} height={viewport.height} aria-hidden="true"><path d={shade} fillRule="evenodd" />{line ? <path className="tutorial-connector" d={line} /> : null}</svg>
    {hole ? <div className="tutorial-spotlight" style={{ left: hole.left, top: hole.top, width: hole.width, height: hole.height }} aria-hidden="true" /> : null}
    <section ref={bubble} className="tutorial-callout" style={{ left: position.left, top: position.top }}>
      <button type="button" className="tutorial-close" aria-label="튜토리얼 닫기" onClick={close}>×</button>
      <div key={index} className="tutorial-copy" aria-live="polite">
        <h2 id="tutorial-title">{index + 1}. {step?.title}</h2>
        <p id="tutorial-description">{step?.description}</p>
        {step?.hint && hole ? <p className="tutorial-hint">{emptyAction ? step.emptyActionFallback?.message : step.hint}</p> : null}
        {step && !hole ? <p className="tutorial-hint">화면을 불러오는 중…</p> : null}
        {satisfied ? <p className="tutorial-hint">{step?.satisfiedHint ?? "조건 충족 · 다음으로 진행"}</p> : null}
        {actionState.phase === "pending" ? <p className="tutorial-hint" role="status">처리 중…</p> : null}
        {actionState.phase === "error" ? <p className="tutorial-error" role="alert">{actionState.message} 같은 동작으로 재시도.</p> : null}
      </div>
      {(!step?.action || satisfied) && actionState.phase !== "pending" ? <footer><button type="button" className="tutorial-next" data-tour-next onClick={next}>{index === steps.length - 1 ? "마치기" : "다음"}</button></footer> : null}
      {emptyAction && step?.emptyActionFallback ? <footer><button type="button" onClick={() => {
        const destination = steps.findIndex(candidate => candidate.id === step.emptyActionFallback?.step);
        if (destination >= 0) setIndex(destination);
      }}>{step.emptyActionFallback.label}</button></footer> : null}
    </section>
  </div>;
  return modalHost?.isConnected ? createPortal(content, modalHost) : content;
}
