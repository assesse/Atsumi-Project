import { FluentIcon } from "./FluentIcon";

type SelectionToolbarProps = {
  /** True only while two or more cards are selected. */
  active: boolean;
  count: number;
  downloadsView: boolean;
  restoreMode?: boolean;
  onAll: () => void;
  onClear: () => void;
  onPrimary: () => void;
  onDelete: () => void;
};

export function SelectionToolbar({ active, count, downloadsView, restoreMode = false, onAll, onClear, onPrimary, onDelete }: SelectionToolbarProps) {
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
            <button type="button" className="text-button primary" onClick={onPrimary}>
              <FluentIcon glyph="\uE896" /> {downloadsView ? "선택 파일 다운로드" : "다운로드"}
            </button>
            <button type="button" className="text-button danger-button" onClick={onDelete}>
              {downloadsView ? (restoreMode ? "복원" : "격리") : "제외"}
            </button>
          </>
        ) : null}
      </div>
    </div>
  );
}
