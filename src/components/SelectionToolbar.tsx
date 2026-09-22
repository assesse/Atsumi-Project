import { FluentIcon } from "./FluentIcon";

type SelectionToolbarProps = {
  /** True only while two or more cards are selected. */
  active: boolean;
  count: number;
  downloadsView: boolean;
  restoreMode?: boolean;
  cancelCount?: number;
  cancelPending?: boolean;
  downloadPending?: boolean;
  onCancelDownloads?: () => void;
  onAll: () => void;
  onClear: () => void;
  onPrimary: () => void;
  onDelete: () => void;
};

export function SelectionToolbar({ active, count, downloadsView, restoreMode = false, cancelCount = 0, cancelPending = false, downloadPending = false, onCancelDownloads, onAll, onClear, onPrimary, onDelete }: SelectionToolbarProps) {
  return (
    <div className="selection-slot">
      <div className={`selection-toolbar${active ? " is-visible" : ""}`} aria-live={active ? "polite" : "off"}>
        {active ? (
          <>
            <strong>{count}개 선택됨</strong>
            <button type="button" className="text-button" onClick={onAll}>
              전체 선택
            </button>
            <button type="button" className="text-button" onClick={onClear}>
              선택 해제
            </button>
            <button type="button" className="text-button primary" disabled={downloadPending} onClick={onPrimary}>
              <FluentIcon glyph="\uE896" /> {downloadsView ? "선택 파일 다운로드" : "다운로드"}
            </button>
            {onCancelDownloads && (cancelCount > 0 || cancelPending) ? (
              <button type="button" className="text-button danger-button" disabled={cancelPending || downloadPending} onClick={onCancelDownloads}
                title="선택한 진행 중 작업만 중단합니다. 완료된 앨범이나 이미 받은 파일은 삭제하지 않습니다.">
                <FluentIcon glyph="\uE71A" /> {cancelPending ? "취소 중…" : `다운로드 취소 · ${cancelCount}개`}
              </button>
            ) : null}
            <button type="button" className="text-button danger-button" disabled={downloadPending} onClick={onDelete}>
              {downloadsView ? (restoreMode ? "복원" : "격리") : "제외"}
            </button>
          </>
        ) : null}
      </div>
    </div>
  );
}
