import atsumiLogo from "../assets/atsumi.svg";
import type { ViewId } from "../core/types";
import { FluentIcon } from "./FluentIcon";

type SideRailProps = {
  view: ViewId;
  collapsed: boolean;
  autoFindCount: number;
  attentionCount: number;
  sourceLabel: string;
  onNavigate: (view: ViewId) => void;
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
  onNavigate,
  onToggle,
}: SideRailProps) {
  return (
    <aside className="sidebar" aria-label="주 메뉴">
      <div className="brand" title="Atsumi Next">
        <img src={atsumiLogo} alt="" />
        <div className="brand-copy">
          <strong>Atsumi</strong>
          <span>Hitomi library</span>
        </div>
      </div>

      <nav className="main-nav">
        {items.map((item) => {
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
