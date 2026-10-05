import { useEffect, useRef } from "react";
import { FluentIcon } from "./FluentIcon";
import { MovingTabs } from "./WindowMotion";

export type ExploreContextTab = {
  id: string;
  label: string;
  page?: number;
  totalPages?: number;
  busy: boolean;
};

type ExploreContextBarProps = {
  tabs: ExploreContextTab[];
  activeId: string;
  onActivate: (id: string) => void;
  onClose: (id: string) => void;
};

export function ExploreContextBar({
  tabs,
  activeId,
  onActivate,
  onClose,
}: ExploreContextBarProps) {
  const tabList = useRef<HTMLDivElement>(null);
  useEffect(() => {
    [...(tabList.current?.querySelectorAll<HTMLElement>('[aria-selected="true"]') ?? [])]
      .find((tab) => !tab.closest("[inert]"))?.scrollIntoView?.({ block: "nearest", inline: "nearest" });
  }, [activeId]);
  const activeIndex = tabs.findIndex((tab) => tab.id === activeId);
  if (activeIndex < 0) return null;

  return (
    <section className="explore-context-bar" aria-label="열린 탐색">
      <div ref={tabList} className="explore-context-tabs" role="tablist" aria-label="탐색 세션">
        <MovingTabs>{tabs.map((tab) => {
          const active = tab.id === activeId;
          const pageDescription = tab.page === undefined
            ? null
            : `${tab.page} / ${Math.max(1, tab.totalPages ?? 1)} 페이지`;
          const accessibleStatus = [tab.busy ? "불러오는 중" : null, pageDescription]
            .filter(Boolean)
            .join(", ");
          return (
            <div className={`explore-context-tab-shell${active ? " is-active" : ""}`} key={tab.id}>
              <button
                type="button"
                className="explore-context-tab"
                role="tab"
                aria-selected={active}
                aria-controls="gallery-viewport"
                aria-busy={tab.busy || undefined}
                aria-label={accessibleStatus ? `${tab.label}, ${accessibleStatus}` : tab.label}
                data-explore-context-id={tab.id}
                onClick={() => onActivate(tab.id)}
              >
                <span>{tab.label}</span>
                {tab.busy ? <small>불러오는 중</small> : null}
              </button>
              <button
                type="button"
                className="explore-context-close"
                aria-label={`${tab.label} 탐색 닫기`}
                title="탐색 닫기"
                onClick={() => onClose(tab.id)}
              >
                <FluentIcon glyph="\uE711" />
              </button>
            </div>
          );
        })}</MovingTabs>
      </div>
    </section>
  );
}
