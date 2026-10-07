import { useEffect, useRef, type ReactNode } from "react";
import { FluentIcon } from "./FluentIcon";

/** Keep routine processing evidence available without expanding the results header. */
export function ResultDetails({ count, busy = false, error = false, children }: {
  count: number; busy?: boolean; error?: boolean; children: ReactNode;
}) {
  const root = useRef<HTMLDetailsElement>(null);
  useEffect(() => {
    const dismiss = (event: PointerEvent) => {
      if (root.current?.open && event.target instanceof Node && !root.current.contains(event.target)) root.current.open = false;
    };
    document.addEventListener("pointerdown", dismiss);
    return () => document.removeEventListener("pointerdown", dismiss);
  }, []);
  return <details ref={root} className="result-details" onKeyDown={event => {
    if (event.key !== "Escape" || !root.current?.open) return;
    event.preventDefault(); event.stopPropagation(); root.current.open = false;
    root.current.querySelector("summary")?.focus();
  }}>
    <summary aria-label={`${count}개 결과 · 처리 상태 보기`}>
      {busy ? <span className="spinner catalog-refresh-spinner" aria-hidden="true" /> : null}
      {error ? <span className="result-details-error" title="처리 오류 확인"><FluentIcon glyph="\uE7BA" /></span> : null}
      <span>{count}개 결과</span><FluentIcon glyph="\uE70D" />
    </summary>
    <div className="result-details-popover">{children}</div>
  </details>;
}
