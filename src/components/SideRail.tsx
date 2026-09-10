import atsumiLogo from "../assets/atsumi.svg";
import { useEffect, useRef, useState } from "react";
import {
  workspaceRegistry,
  workspaces,
  type ContentSource,
  type WorkspaceNavigationItem,
  type WorkspaceViewId,
} from "../app/workspaceRegistry";
import { FluentIcon } from "./FluentIcon";

type SideRailProps<Source extends ContentSource> = {
  view: WorkspaceViewId<Source>;
  collapsed: boolean;
  autoFindCount: number;
  attentionCount: number;
  sourceLabel: string;
  source: Source;
  onNavigate: (view: WorkspaceViewId<Source>) => void;
  onSourceChange: (source: ContentSource) => void;
  onToggle: () => void;
};

export function SideRail<Source extends ContentSource>({
  view,
  collapsed,
  autoFindCount,
  attentionCount,
  sourceLabel,
  source,
  onNavigate,
  onSourceChange,
  onToggle,
}: SideRailProps<Source>) {
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
  const workspace = workspaceRegistry[source];
  const visibleItems = workspace.navigation as readonly WorkspaceNavigationItem<WorkspaceViewId<Source>>[];
  const badgeCounts = { autoFindCount, attentionCount };
  return (
    <aside ref={railRef} className="sidebar" aria-label="주 메뉴">
      <button
        type="button"
        className={`brand${sourceMenuOpen ? " is-open" : ""}`}
        title="Atsumi 소스 전환"
        aria-label={`현재 ${workspace.label} 모드. 소스 전환`}
        aria-haspopup="menu"
        aria-expanded={sourceMenuOpen}
        onClick={() => setSourceMenuOpen((open) => !open)}
      >
        <img src={atsumiLogo} alt="" />
        <div className="brand-copy">
          <strong>Atsumi</strong>
          <span>{workspace.subtitle}</span>
        </div>
        <FluentIcon glyph="\uE70D" className="brand-chevron" />
      </button>
      {sourceMenuOpen ? (
        <div className="source-switcher" role="menu" aria-label="콘텐츠 소스 선택">
          {workspaces.map((option) => (
            <button
              key={option.id}
              type="button"
              role="menuitemradio"
              aria-checked={source === option.id}
              className={source === option.id ? "is-active" : ""}
              onClick={() => { onSourceChange(option.id); setSourceMenuOpen(false); }}
            >
              <strong>{option.label}</strong><span>{option.description}</span>
            </button>
          ))}
        </div>
      ) : null}

      <nav className="main-nav">
        {visibleItems.map((item) => {
          const count = item.badgeKey ? badgeCounts[item.badgeKey] : 0;
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
                <span className={`nav-count${item.badgeWarning ? " warning" : ""}`}>{count}</span>
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
