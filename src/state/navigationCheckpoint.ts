import type { SearchRequest } from "../api/contracts";
import type { DownloadFilter, Language, SearchSort, ViewId, DetailState } from "../core/types";
import type { DetailPosition } from "./detailPositions";
export type NavigationOrigin = { view: ViewId; contextId: string | null; detailId: number | null };
export type SavedExploreTab = { id: string; label: string; displayValue: string; request: SearchRequest;
  queryId: string | null; page: number; scrollTop: number; origin?: NavigationOrigin };
export type NavigationCheckpoint = { version: 1; savedAt: number; view: ViewId; downloadsFilter: DownloadFilter;
  activeTab: string | null; tabs: SavedExploreTab[]; detail?: DetailState;
  detailOrigins?: [number, NavigationOrigin][]; detailPositions?: [number, DetailPosition][] };
const KEY = "atsumi.navigation-checkpoint.v1";
const LIMIT = 128 * 1024;
const views = ["explore", "auto-find", "downloads"];
const sorts = ["recent", "popular_today", "popular_week", "popular_month", "popular_year", "random"];
const text = (s: unknown, max = 2048): string => typeof s === "string" ? s.slice(0, max) : "";
const integer = (n: unknown, max: number) => typeof n === "number" && Number.isFinite(n) ? Math.max(1, Math.min(max, Math.floor(n))) : 1;
function sanitize(value: unknown): NavigationCheckpoint | null {
  if (!value || typeof value !== "object") return null;
  const v=value as Partial<NavigationCheckpoint>;
  if (v.version!==1 || !Array.isArray(v.tabs) || !Number.isFinite(v.savedAt) || Date.now()-v.savedAt!>86_400_000) return null;
  const tabs:SavedExploreTab[]=[];
  const origin = (o: NavigationOrigin | undefined): NavigationOrigin | undefined => o && typeof o === "object" && views.includes(o.view) ? {
    view: o.view, contextId: typeof o.contextId === "string" ? text(o.contextId,80) : null,
    detailId: Number.isSafeInteger(o.detailId) && o.detailId! > 0 ? o.detailId : null,
  } : undefined;
  for (const candidate of v.tabs.slice(0,64)) {
    if (!candidate || typeof candidate!=="object" || !candidate.request || typeof candidate.request!=="object") continue;
    const r=candidate.request;
    const tags=(a:unknown) => Array.isArray(a) ? a.filter((t):t is string => typeof t === "string").slice(0,64).map((t)=>text(t,128)) : [];
    tabs.push({ id:text(candidate.id,80),label:text(candidate.label,128),displayValue:text(candidate.displayValue),
      ...(origin(candidate.origin) ? { origin: origin(candidate.origin) } : {}),
      queryId:typeof candidate.queryId === "string" ? text(candidate.queryId,128) : null,
      page:integer(candidate.page,1_000_000),scrollTop:Math.max(0,Math.min(10_000_000,Number(candidate.scrollTop)||0)),
      request: { text:text(r.text),includeTags:tags(r.includeTags),excludeTags:tags(r.excludeTags),
        languages:(Array.isArray(r.languages) ? r.languages : []).filter((l):l is Language => ["korean","japanese","chinese","english"].includes(l)).slice(0,4),
        sort:(sorts.includes(r.sort) ? r.sort : "recent") as SearchSort,pageSize:integer(r.pageSize,200) } });
  }
  return { version:1,savedAt:v.savedAt!,view:(views.includes(v.view!) ? v.view : "explore") as ViewId,
    downloadsFilter:(["all","active","review","failed","complete"].includes(v.downloadsFilter!) ? v.downloadsFilter : "all") as DownloadFilter,
    activeTab:typeof v.activeTab === "string" && tabs.some((t)=>t.id===v.activeTab) ? v.activeTab : tabs[0]?.id ?? null,tabs,
    ...(v.detail && Array.isArray(v.detail.tabs) ? { detail: {
      tabs: [...new Set(v.detail.tabs.filter((id)=>Number.isSafeInteger(id) && id > 0))].slice(0,64),
      activeId: v.detail.tabs.includes(v.detail.activeId!) ? v.detail.activeId : null, minimized: Boolean(v.detail.minimized),
    } } : {}),
    ...(Array.isArray(v.detailOrigins) ? { detailOrigins: v.detailOrigins.slice(-96).flatMap((pair): [number, NavigationOrigin][] => {
      if (!Array.isArray(pair) || !Number.isSafeInteger(pair[0]) || pair[0] < 1) return [];
      const clean = origin(pair[1]); return clean ? [[pair[0], clean]] : [];
    }) } : {}),
    ...(Array.isArray(v.detailPositions) ? { detailPositions: v.detailPositions.slice(-96).flatMap((pair): [number, DetailPosition][] =>
      Array.isArray(pair) && Number.isSafeInteger(pair[0]) && pair[0] > 0 && pair[1] && Number.isFinite(pair[1].scrollTop)
        ? [[pair[0], { scrollTop: Math.max(0, Math.min(10_000_000, pair[1].scrollTop)), previewStart: integer(pair[1].previewStart,1_000_000) }]] : []) } : {}),
  };
}
export function readNavigationCheckpoint(storage: Pick<Storage,"getItem">=sessionStorage): NavigationCheckpoint | null {
  try { const raw=storage.getItem(KEY); return raw && raw.length<=LIMIT ? sanitize(JSON.parse(raw)) : null; } catch { return null; }
}
export function writeNavigationCheckpoint(value:NavigationCheckpoint, storage:Pick<Storage,"setItem">=sessionStorage): void {
  try {
    const clean=sanitize(value); if (!clean) return;
    let raw=JSON.stringify(clean);
    while (raw.length>LIMIT && clean.tabs.length) {
      const removable=clean.tabs.findIndex((tab)=>tab.id!==clean.activeTab);
      if (removable<0) return;
      clean.tabs.splice(removable,1); raw=JSON.stringify(clean);
    }
    storage.setItem(KEY,raw);
  } catch { /* Storage disabled/full must not affect navigation. */ }
}
