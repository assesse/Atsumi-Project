import atsumiLogo from "../assets/atsumi.svg";
import { useEffect, useRef, useState } from "react";
import type { ContentSource, ViewId } from "../core/types";
import { FluentIcon } from "./FluentIcon";

type SideRailProps = {
  view: ViewId;
  collapsed: boolean;
  autoFindCount: number;
  attentionCount: number;
  sourceLabel: string;
  source: ContentSource;
  onNavigate: (view: ViewId) => void;
  onSourceChange: (source: ContentSource) => void;
  onToggle: () => void;
};

const items: Array<{ view: ViewId; label: string; icon: string }> = [
  { view: "explore", label: "Explore", icon: "\uE80F" },
  { view: "auto-find", label: "Auto Find", icon: "\uE735" },
  { view: "downloads", label: "Downloads", icon: "\uE896" },
];

export function SideRail({
  view,
  collapsed,
  autoFindCount,
  attentionCount,
  sourceLabel,
  source,
  onNavigate,
  onSourceChange,
  onToggle,
}: SideRailProps) {
  const [sourceMenuOpen, setSourceMenuOpen] = useState(false);
  const railRef = useRef<HTMLElement>(null);
  useEffect(() => {
    if (!sourceMenuOpen) return;
    const close = (event: KeyboardEvent) => {
      if (event.key === "Escape") setSourceMenuOpen(false);
    };
    window.addEventListener("keydown", close);
    const pointerDown = (event: PointerEvent) => {
      if (event.target instanceof Node && !railRef.current?.contains(event.target)) setSourceMenuOpen(false);
    };
    window.addEventListener("pointerdown", pointerDown);
    return () => {
      window.removeEventListener("keydown", close);
      window.removeEventListener("pointerdown", pointerDown);
    };
  }, [sourceMenuOpen]);
  const visibleItems = source === "danbooru" ? items.filter((item) => item.view !== "auto-find") : items;
  return (
    <aside ref={railRef} className="sidebar" aria-label="주 메뉴">
      <button
        type="button"
        className={`brand${sourceMenuOpen ? " is-open" : ""}`}
        title="Atsumi 소스 전환"
        aria-label={`현재 ${source === "hitomi" ? "Hitomi" : "Danbooru"} 모드. 소스 전환`}
        aria-haspopup="menu"
        aria-expanded={sourceMenuOpen}
        onClick={() => setSourceMenuOpen((open) => !open)}
      >
        <img src={atsumiLogo} alt="" />
        <div className="brand-copy">
          <strong>Atsumi</strong>
          <span>{source === "hitomi" ? "Hitomi library" : "Danbooru posts"}</span>
        </div>
        <FluentIcon glyph="\uE70D" className="brand-chevron" />
      </button>
      {sourceMenuOpen ? (
        <div className="source-switcher" role="menu" aria-label="콘텐츠 소스 선택">
          <button
            type="button"
            role="menuitemradio"
            aria-checked={source === "hitomi"}
            className={source === "hitomi" ? "is-active" : ""}
            onClick={() => { onSourceChange("hitomi"); setSourceMenuOpen(false); }}
          >
            <strong>Hitomi</strong><span>앨범 탐색·다운로드·중복 검토</span>
          </button>
          <button
            type="button"
            role="menuitemradio"
            aria-checked={source === "danbooru"}
            className={source === "danbooru" ? "is-active" : ""}
            onClick={() => { onSourceChange("danbooru"); setSourceMenuOpen(false); }}
          >
            <strong>Danbooru</strong><span>post 검색·미리보기·원본 보관</span>
          </button>
        </div>
      ) : null}

      <nav className="main-nav">
        {visibleItems.map((item) => {
          const count = item.view === "auto-find" ? autoFindCount : item.view === "downloads" ? attentionCount : 0;
          return (
            <button
              key={item.view}
              type="button"
              className={`nav-item${view === item.view ? " is-active" : ""}`}
              aria-current={view === item.view ? "page" : undefined}
              onClick={() => onNavigate(item.view)}
            >
              <FluentIcon glyph={item.icon} />
              <span className="nav-label">{item.label}</span>
              {count > 0 ? (
                <span className={`nav-count${item.view === "downloads" ? " warning" : ""}`}>{count}</span>
              ) : null}
            </button>
          );
        })}
      </nav>

      <div className="sidebar-foot">
        <span className="live-indicator">
          <i />
          <span className="nav-label">{sourceLabel}</span>
        </span>
        <button
          type="button"
          className="icon-button sidebar-toggle"
          title={collapsed ? "메뉴 펼치기" : "메뉴 접기"}
          aria-label={collapsed ? "메뉴 펼치기" : "메뉴 접기"}
          onClick={onToggle}
        >
          <FluentIcon glyph={collapsed ? "\uE76C" : "\uE76B"} />
        </button>
      </div>
    </aside>
  );
}
