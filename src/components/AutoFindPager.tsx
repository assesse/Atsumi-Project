type AutoFindPagerProps = {
  page: number;
  totalPages: number;
  totalItems: number;
  onPageChange: (page: number) => void;
};

export function AutoFindPager({ page, totalPages, totalItems, onPageChange }: AutoFindPagerProps) {
  if (totalPages <= 1) return null;

  return (
    <nav className="pager auto-find-pager" aria-label="Auto Find 페이지">
      <button
        type="button"
        className="text-button"
        disabled={page <= 1}
        onClick={() => onPageChange(page - 1)}
      >이전</button>
      <span><strong>{page} / {totalPages}</strong> · 전체 {totalItems.toLocaleString()}개</span>
      <button
        type="button"
        className="text-button"
        disabled={page >= totalPages}
        onClick={() => onPageChange(page + 1)}
      >다음</button>
    </nav>
  );
}
