import { useEffect, useId, useLayoutEffect, useMemo, useRef, useState, type HTMLAttributes, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { hasKanji, safeMetadataText } from "./japaneseReading";
import { isReadableMetadataOriginal, metadataLines, METADATA_LABELS, type MetadataKind, type MetadataLine } from "./metadataTranslation";
import { pixivReadings } from "./readingService";
import type { PixivTagTranslation } from "./model";
import "./PixivMetadataText.css";

type Props = Omit<HTMLAttributes<HTMLElement>, "children" | "title"> & {
  as?: "span" | "strong" | "button"; text: string; kind: MetadataKind; hints?: PixivTagTranslation[];
  private?: boolean; disabled?: boolean; children?: ReactNode;
};

function visibleOwner(node: HTMLElement | null): boolean {
  if (!node?.isConnected || node.closest('[hidden],[inert],[aria-hidden="true"]')) return false;
  for (let current: HTMLElement | null = node; current; current = current.parentElement) {
    const style = getComputedStyle(current);
    if (style.display === "none" || style.visibility === "hidden" || style.opacity === "0") return false;
  }
  const modal = [...document.querySelectorAll('dialog[open],[aria-modal="true"]')].filter(element => !element.closest("[hidden],[inert]") && getComputedStyle(element).display !== "none").at(-1);
  return !modal || modal.contains(node);
}

export function PixivMetadataText({ as: Element = "span", text, kind, hints, private: hidden = false, disabled, children, ...props }: Props) {
  const trigger = useRef<HTMLElement | null>(null), timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const id = useId(), [openTarget, setOpenTarget] = useState<string>(), [reading, setReading] = useState<MetadataLine | undefined>(), [loading, setLoading] = useState(false);
  const base = useMemo(() => metadataLines(text, kind, hints), [text, kind, hints]);
  const targetKey = `${kind}:${text}`, canRead = !hidden && safeMetadataText(text), shown = openTarget === targetKey && canRead;
  const hide = () => { clearTimeout(timer.current); setOpenTarget(undefined); };
  const show = () => {
    clearTimeout(timer.current);
    if (!canRead) return;
    timer.current = setTimeout(() => {
      if (visibleOwner(trigger.current)) setOpenTarget(targetKey);
    }, 300);
  };
  useEffect(() => { hide(); setReading(undefined); return () => clearTimeout(timer.current); }, [targetKey, hidden]);
  useEffect(() => {
    setReading(undefined); setLoading(false);
    // A meaning does not replace a reading. Resolve only missing Kanji readings,
    // on deliberate hover/focus, using the same bounded local worker.
    if (!shown || !hasKanji(text) || base.some(line => line.label === METADATA_LABELS.reading)) return;
    const controller = new AbortController(); setLoading(true);
    void pixivReadings.read(text, controller.signal).then(result => {
      if (controller.signal.aborted) return;
      setReading(result ? { label: METADATA_LABELS.reading, text: result.text, language: "en", note: result.partial ? "일부 추정" : "추정" } : undefined);
      setLoading(false);
    });
    return () => controller.abort();
  }, [shown, text, kind, hints, base]);
  useEffect(() => {
    if (!shown) return;
    const dismiss = () => hide();
    const key = () => hide(); // Dismiss only; the existing shortcut still receives the event.
    const checkOwner = () => { if (!visibleOwner(trigger.current)) hide(); };
    const observer = new MutationObserver(checkOwner);
    // Watch only this visible tooltip's ancestor attributes and top-level modal
    // portals, never the entire live chat/card subtree or every card at startup.
    for (let node = trigger.current; node; node = node.parentElement) observer.observe(node, { attributes: true, attributeFilter: ["hidden", "inert", "aria-hidden", "class", "style", "data-controls-visible"] });
    observer.observe(document.body, { childList: true, attributes: true, attributeFilter: ["hidden", "inert", "aria-hidden", "class", "style"] });
    document.addEventListener("transitionend", checkOwner, true);
    window.addEventListener("blur", dismiss); document.addEventListener("scroll", dismiss, true); document.addEventListener("keydown", key, true);
    document.addEventListener("visibilitychange", dismiss);
    return () => { observer.disconnect(); document.removeEventListener("transitionend", checkOwner, true); window.removeEventListener("blur", dismiss); document.removeEventListener("scroll", dismiss, true); document.removeEventListener("keydown", key, true); document.removeEventListener("visibilitychange", dismiss); };
  }, [shown]);
  return <>
    <Element {...props} ref={node => { trigger.current = node; }} type={Element === "button" ? "button" : undefined} disabled={Element === "button" ? disabled : undefined}
      data-pixiv-metadata={hidden ? undefined : kind} aria-describedby={shown ? [props["aria-describedby"], id].filter(Boolean).join(" ") : props["aria-describedby"]}
      onMouseEnter={event => { show(); props.onMouseEnter?.(event); }} onMouseLeave={event => { hide(); props.onMouseLeave?.(event); }}
      onFocus={event => { show(); props.onFocus?.(event); }} onBlur={event => { hide(); props.onBlur?.(event); }}
      onPointerDown={event => { hide(); props.onPointerDown?.(event); }}>
      {children ?? (hidden ? kind === "artist" ? "작가" : "작품" : text)}
    </Element>
    {shown && <MetadataTooltip id={id} trigger={trigger.current} original={text} lines={reading ? [...base, reading] : base} loading={loading} />}
  </>;
}

export function MetadataTooltip({ id, trigger, original, lines, loading }: { id: string; trigger: HTMLElement | null; original: string; lines: MetadataLine[]; loading: boolean }) {
  const ref = useRef<HTMLDivElement>(null), [position, setPosition] = useState({ left: 8, top: 8, ready: false });
  useLayoutEffect(() => {
    const place = () => {
      if (!trigger?.isConnected || !ref.current) return;
      const anchor = trigger.getBoundingClientRect(), box = ref.current.getBoundingClientRect(), width = window.innerWidth, height = window.innerHeight;
      const left = Math.max(8, Math.min(anchor.left + anchor.width / 2 - box.width / 2, width - box.width - 8));
      const top = anchor.top >= box.height + 16 ? anchor.top - box.height - 8 : Math.max(8, Math.min(anchor.bottom + 8, height - box.height - 8));
      setPosition({ left, top, ready: true });
    };
    place(); window.addEventListener("resize", place);
    return () => window.removeEventListener("resize", place);
  }, [trigger, original, lines, loading]);
  const owner = trigger?.closest("dialog[open]") ?? document.body;
  return createPortal(<div id={id} ref={ref} role="tooltip" className="pixiv-metadata-tooltip" style={{ left: position.left, top: position.top, visibility: position.ready ? "visible" : "hidden" }}>
    <div className="pixiv-metadata-original">{original}</div>
    {lines.map((line, index) => <div className="pixiv-metadata-line" key={`${line.label}-${index}`}><span>{line.label}</span><span lang={line.language}>{line.text}{line.note && <small className="pixiv-metadata-note" lang="ko"> ({line.note})</small>}</span></div>)}
    {loading && <div className="pixiv-metadata-status">발음 확인 중…</div>}
    {!loading && lines.length === 0 && !isReadableMetadataOriginal(original) && <div className="pixiv-metadata-status">번역·발음 정보 없음</div>}
  </div>, owner);
}
