import { FluentIcon } from "./FluentIcon";

export type ExploreContextTab = {
  id: string;
  label: string;
  page?: number;
  totalPages?: number;
  root: boolean;
  busy: boolean;
};

type ExploreContextBarProps = {
  tabs: ExploreContextTab[];
  activeId: string;
  onActivate: (id: string) => void;
  onBack: () => void;
  onClose: (id: string) => void;
};

export function ExploreContextBar({
  tabs,
  activeId,
  onActivate,
  onBack,
  onClose,
}: ExploreContextBarProps) {
  const activeIndex = tabs.findIndex((tab) => tab.id === activeId);
  if (tabs.length < 2 || activeIndex < 0) return null;

  return (
    <section className="explore-context-bar" aria-label="열린 탐색">
      <button
        type="button"
        className="explore-context-back"
        disabled={activeIndex <= 0}
        aria-label="이전 탐색으로 돌아가기"
        title="이전 탐색으로 돌아가기"
        onClick={onBack}
      >
        <FluentIcon glyph="\uE72B" />
        이전 탐색
      </button>
      <div className="explore-context-tabs" role="tablist" aria-label="탐색 세션">
        {tabs.map((tab) => {
          const active = tab.id === activeId;
          return (
            <div className={`explore-context-tab-shell${active ? " is-active" : ""}`} key={tab.id}>
              <button
                type="button"
                className="explore-context-tab"
                role="tab"
                aria-selected={active}
                aria-controls="gallery-viewport"
                data-explore-context-id={tab.id}
                onClick={() => onActivate(tab.id)}
              >
                <span>{tab.label}</span>
                {tab.busy ? (
                  <small>불러오는 중</small>
                ) : tab.page !== undefined ? (
                  <small>{tab.page} / {Math.max(1, tab.totalPages ?? 1)}</small>
                ) : null}
              </button>
              {!tab.root ? (
                <button
                  type="button"
                  className="explore-context-close"
                  aria-label={`${tab.label} 탐색 닫기`}
                  title="탐색 닫기"
                  onClick={() => onClose(tab.id)}
                >
                  <FluentIcon glyph="\uE711" />
                </button>
              ) : null}
            </div>
          );
        })}
      </div>
    </section>
  );
}
