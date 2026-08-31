import type { GalleryDisplayMode } from "../core/types";

type GalleryDisplayModeControlProps = {
  value: GalleryDisplayMode;
  onChange: (mode: GalleryDisplayMode) => void;
};

export function GalleryDisplayModeControl({ value, onChange }: GalleryDisplayModeControlProps) {
  return (
    <div className="segmented gallery-display-mode-control" role="group" aria-label="앨범 카드 표시 방식">
      <button
        type="button"
        aria-pressed={value === "detail"}
        className={value === "detail" ? "is-active" : ""}
        onClick={() => onChange("detail")}
      >상세</button>
      <button
        type="button"
        aria-pressed={value === "compact"}
        className={value === "compact" ? "is-active" : ""}
        onClick={() => onChange("compact")}
      >요약</button>
    </div>
  );
}
