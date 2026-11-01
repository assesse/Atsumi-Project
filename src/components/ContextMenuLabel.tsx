import type { ReactNode } from "react";
import "./ContextMenuLabel.css";

const paths = {
  open: "M4 4h16v16H4zM8 4v16M11 8h5m-5 4h5",
  background: "M8 3h13v13M3 8h13v13H3zM7 14h5m-2.5-2.5v5",
  download: "M12 3v12m-5-5 5 5 5-5M4 17v4h16v-4",
  cancel: "m6 6 12 12M6 18 18 6",
  check: "m4 12 5 5L20 6",
  favorite: "m12 3 2.8 5.7 6.2.9-4.5 4.4 1.1 6.2-5.6-3-5.6 3 1.1-6.2L3 9.6l6.2-.9Z",
  folder: "M3 7V4h6l2 3h10v13H3V7Zm0 3h18",
  external: "M14 3h7v7m0-7L10 14M10 3H3v18h18v-7",
  search: "M17 10a7 7 0 1 1-14 0 7 7 0 0 1 14 0Zm-2 5 6 6",
  play: "m7 4 14 8-14 8Z",
  restore: "M4 5v6h6M4 11a8 8 0 1 1 2 7",
  remove: "M3 6h18M9 6V3h6v3M5 6l1 15h12l1-15M10 10v7m4-7v7",
  exclude: "M20 12a8 8 0 1 1-16 0 8 8 0 0 1 16 0ZM6.3 6.3l11.4 11.4",
  review: "M3 4h7v16H3zM14 4h7v16h-7zM6 8h1m10 0h1M6 12h1m10 0h1",
  creator: "M16 7a4 4 0 1 1-8 0 4 4 0 0 1 8 0ZM4 21v-2a8 6 0 0 1 16 0v2",
} as const;

export type ContextMenuIcon = keyof typeof paths;

// Stable action IDs, not translated labels. Less frequent informational actions
// keep the same empty column; an explicit override can reflect a state change.
const cardIcons: Readonly<Record<string, ContextMenuIcon>> = {
  open: "open", detail: "open", background: "background", folder: "folder",
  artifact: "external", source: "external", download: "download", bookmark: "favorite",
  favorite: "favorite", review: "review", "internal-review": "review", exclude: "exclude",
  archive: "exclude", remove: "remove", restore: "restore", retry: "restore",
  "exclusion-restore": "restore", "exclusion-retained": "open",
};

export function cardContextMenuIcon(id: string): ContextMenuIcon | undefined {
  return Object.hasOwn(cardIcons, id) ? cardIcons[id] : undefined;
}

export function ContextMenuLabel({ icon, children }: { icon?: ContextMenuIcon | null; children: ReactNode }) {
  return <span className="context-menu-label">
    <span className="context-menu-icon" aria-hidden="true" data-menu-icon={icon ?? undefined}>
      {icon ? <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" focusable="false"><path d={paths[icon]} /></svg> : null}
    </span>
    <span className="context-menu-text">{children}</span>
  </span>;
}
