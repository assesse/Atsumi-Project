import { useEffect, useRef, useState, type FormEvent, type KeyboardEvent } from "react";

type AutoFindPagerProps = {
  page: number;
  totalPages: number;
  onPageChange: (page: number) => void;
  ariaLabel?: string;
  className?: string;
  busy?: boolean;
};

export function AutoFindPager({
  page,
  totalPages,
  onPageChange,
  ariaLabel = "Auto Find 페이지",
  className = "auto-find-pager",
  busy = false,
}: AutoFindPagerProps) {
  const [draft, setDraft] = useState(String(page));
  const suppressNextBlurCommit = useRef(false);

  useEffect(() => {
    setDraft(String(page));
  }, [page]);

  if (totalPages <= 1) return null;

  const commitDraft = () => {
    if (!draft.trim()) {
      setDraft(String(page));
      return;
    }
    const parsed = Number(draft);
    if (!Number.isFinite(parsed)) {
      setDraft(String(page));
      return;
    }
    const target = Math.max(1, Math.min(totalPages, Math.floor(parsed)));
    setDraft(String(target));
    if (target !== page) onPageChange(target);
  };

  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!busy) commitDraft();
  };

  const handleInputKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === "Enter") {
      event.preventDefault();
      if (!busy) commitDraft();
      return;
    }
    if (event.key !== "Escape") return;
    event.preventDefault();
    suppressNextBlurCommit.current = true;
    setDraft(String(page));
    event.currentTarget.blur();
    queueMicrotask(() => { suppressNextBlurCommit.current = false; });
  };

  return (
    <nav className={`pager ${className}`} aria-label={ariaLabel}>
      <button
        type="button"
        className="text-button"
        disabled={busy || page <= 1}
        onClick={() => onPageChange(page - 1)}
      >이전</button>
      <form className="pager-page-form" onSubmit={submit}>
        <label>
          <span className="sr-only">{ariaLabel} 번호 직접 입력</span>
          <input
            className="pager-page-input"
            type="number"
            inputMode="numeric"
            min={1}
            max={totalPages}
            step={1}
            value={draft}
            disabled={busy}
            aria-label={`${ariaLabel} 번호 직접 입력`}
            onChange={(event) => setDraft(event.currentTarget.value)}
            onBlur={() => {
              if (suppressNextBlurCommit.current) {
                suppressNextBlurCommit.current = false;
                return;
              }
              if (!busy) commitDraft();
            }}
            onKeyDown={handleInputKeyDown}
          />
        </label>
        <span className="pager-page-total" aria-hidden="true">/ {totalPages}</span>
        <span className="sr-only">현재 {page} / {totalPages}</span>
        <button type="submit" className="sr-only" disabled={busy}>이동</button>
      </form>
      {busy ? <span className="pager-busy" role="status">불러오는 중</span> : null}
      <button
        type="button"
        className="text-button"
        disabled={busy || page >= totalPages}
        onClick={() => onPageChange(page + 1)}
      >다음</button>
    </nav>
  );
}
