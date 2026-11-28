import { useId, useLayoutEffect, useRef } from "react";
import type { AutoFindRefreshMode } from "../api/contracts";
import { FluentIcon } from "./FluentIcon";
import "./AutoFindRefreshDialog.css";

export function AutoFindRefreshDialog({ onChoose, onClose }: { onChoose(mode: AutoFindRefreshMode): void; onClose(): void }) {
  const ref = useRef<HTMLDialogElement>(null);
  const heading = useId();
  useLayoutEffect(() => {
    const dialog = ref.current;
    if (!dialog) return;
    if (typeof dialog.showModal === "function") dialog.showModal();
    else dialog.setAttribute("open", "");
    return () => { if (dialog.open) dialog.close?.(); };
  }, []);
  return <dialog ref={ref} className="auto-find-refresh-dialog" aria-labelledby={heading} onKeyDown={event => event.stopPropagation()}
    onCancel={event => { event.preventDefault(); onClose(); }}>
    <header><h2 id={heading}>작가·그룹 갱신</h2><button type="button" className="icon-button small" aria-label="닫기" onClick={onClose}>×</button></header>
    <div className="auto-find-refresh-options">
      <button type="button" autoFocus onClick={() => onChoose("incremental")}>
        <FluentIcon glyph="" /><strong>새 작품만 추가</strong><small>마지막 탐색 이후</small>
      </button>
      <button type="button" onClick={() => onChoose("full")}>
        <FluentIcon glyph="" /><strong>전체 탐색</strong><small>처음부터 다시 확인</small>
      </button>
    </div>
  </dialog>;
}
