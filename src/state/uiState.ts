import type {
  DownloadFilter,
  GalleryId,
  Language,
  SearchSort,
  UiState,
  ViewId,
} from "../core/types";

const searchState = () => ({
  draft: "",
  committed: "",
  languages: ["korean"] as Language[],
  suggestionsOpen: false,
  activeSuggestion: null,
});

export const initialUiState: UiState = {
  view: "explore",
  railCollapsed: false,
  search: {
    explore: searchState(),
    "auto-find": searchState(),
    downloads: searchState(),
  },
  exploreSort: "recent",
  downloadsFilter: "all",
  grouping: { "auto-find": "all", downloads: "all" },
  selection: { ids: new Set(), anchorId: null },
  detail: { tabs: [], activeId: null, minimized: false },
  overlays: {
    activityOpen: false,
    settingsOpen: false,
    reviewGalleryId: null,
    exitConfirmOpen: false,
  },
};

export type UiAction =
  | { type: "navigate"; view: ViewId }
  | { type: "rail.toggle" }
  | { type: "search.draft"; view: ViewId; value: string }
  | { type: "search.suggestions"; view: ViewId; open: boolean; active?: number | null }
  | { type: "search.commit"; view: ViewId; value?: string }
  | { type: "search.languages"; view: ViewId; languages: Language[] }
  | { type: "sort.set"; sort: SearchSort }
  | { type: "downloads.filter"; filter: DownloadFilter }
  | { type: "grouping.set"; view: "auto-find" | "downloads"; grouping: "all" | "day" | "artist" }
  | {
      type: "selection.click";
      id: GalleryId;
      visibleIds: GalleryId[];
      ctrl: boolean;
      shift: boolean;
    }
  | {
      type: "selection.range";
      anchorId: GalleryId;
      id: GalleryId;
      visibleIds: GalleryId[];
    }
  | { type: "selection.clear" }
  | { type: "selection.retain"; ids: GalleryId[] }
  | { type: "selection.restore"; ids: GalleryId[]; anchorId: GalleryId | null }
  | { type: "selection.all"; ids: GalleryId[] }
  | { type: "detail.open"; id: GalleryId; parentId?: GalleryId }
  | { type: "detail.activate"; id: GalleryId }
  | { type: "detail.close"; id: GalleryId }
  | { type: "detail.closeAll" }
  | { type: "detail.minimize"; minimized: boolean }
  | { type: "overlay.activity"; open: boolean }
  | { type: "overlay.settings"; open: boolean }
  | { type: "overlay.review"; galleryId: GalleryId | null }
  | { type: "overlay.exit"; open: boolean };

function updateSearch(
  state: UiState,
  view: ViewId,
  patch: Partial<UiState["search"][ViewId]>,
): UiState {
  return {
    ...state,
    search: {
      ...state.search,
      [view]: { ...state.search[view], ...patch },
    },
  };
}

function selectFromClick(
  state: UiState,
  action: Extract<UiAction, { type: "selection.click" }>,
): UiState {
  const selected = new Set(state.selection.ids);
  let anchorId = state.selection.anchorId;

  if (action.shift && anchorId !== null) {
    const start = action.visibleIds.indexOf(anchorId);
    const end = action.visibleIds.indexOf(action.id);
    if (start >= 0 && end >= 0) {
      const from = Math.min(start, end);
      const to = Math.max(start, end);
      for (const id of action.visibleIds.slice(from, to + 1)) selected.add(id);
    }
  } else if (action.ctrl) {
    if (selected.has(action.id)) selected.delete(action.id);
    else selected.add(action.id);
    anchorId = action.id;
  } else {
    const soleCardWasClickedAgain = selected.size === 1 && selected.has(action.id);
    selected.clear();
    if (soleCardWasClickedAgain) anchorId = null;
    else {
      selected.add(action.id);
      anchorId = action.id;
    }
  }

  return { ...state, selection: { ids: selected, anchorId } };
}

function closeDetail(state: UiState, id: GalleryId): UiState {
  const index = state.detail.tabs.indexOf(id);
  const tabs = state.detail.tabs.filter((tabId) => tabId !== id);
  let activeId = state.detail.activeId;
  if (activeId === id) activeId = tabs[index] ?? tabs[index - 1] ?? null;
  return {
    ...state,
    detail: {
      tabs,
      activeId,
      minimized: tabs.length ? state.detail.minimized : false,
    },
  };
}

export function uiReducer(state: UiState, action: UiAction): UiState {
  switch (action.type) {
    case "navigate":
      return {
        ...state,
        view: action.view,
        selection: { ids: new Set(), anchorId: null },
        search: {
          ...state.search,
          [state.view]: { ...state.search[state.view], suggestionsOpen: false, activeSuggestion: null },
        },
      };
    case "rail.toggle":
      return { ...state, railCollapsed: !state.railCollapsed };
    case "search.draft":
      return updateSearch(state, action.view, { draft: action.value, activeSuggestion: null });
    case "search.suggestions":
      return updateSearch(state, action.view, {
        suggestionsOpen: action.open,
        activeSuggestion: action.active ?? null,
      });
    case "search.commit": {
      const value = action.value ?? state.search[action.view].draft;
      const next = updateSearch(state, action.view, {
        draft: value,
        committed: value.trim(),
        suggestionsOpen: false,
        activeSuggestion: null,
      });
      return { ...next, selection: { ids: new Set(), anchorId: null } };
    }
    case "search.languages": {
      const next = updateSearch(state, action.view, { languages: action.languages });
      return { ...next, selection: { ids: new Set(), anchorId: null } };
    }
    case "sort.set":
      return { ...state, exploreSort: action.sort };
    case "downloads.filter":
      return {
        ...state,
        downloadsFilter: action.filter,
        selection: { ids: new Set(), anchorId: null },
      };
    case "grouping.set":
      return { ...state, grouping: { ...state.grouping, [action.view]: action.grouping } };
    case "selection.click":
      return selectFromClick(state, action);
    case "selection.range": {
      const start = action.visibleIds.indexOf(action.anchorId);
      const end = action.visibleIds.indexOf(action.id);
      if (start < 0 || end < 0) return state;
      const ids = new Set(state.selection.ids);
      const from = Math.min(start, end);
      const to = Math.max(start, end);
      for (const id of action.visibleIds.slice(from, to + 1)) ids.add(id);
      return { ...state, selection: { ids, anchorId: action.anchorId } };
    }
    case "selection.clear":
      return { ...state, selection: { ids: new Set(), anchorId: null } };
    case "selection.retain": {
      const allowed = new Set(action.ids);
      const ids = new Set([...state.selection.ids].filter((id) => allowed.has(id)));
      const anchorId = state.selection.anchorId !== null && allowed.has(state.selection.anchorId)
        ? state.selection.anchorId
        : null;
      if (ids.size === state.selection.ids.size && anchorId === state.selection.anchorId) return state;
      return { ...state, selection: { ids, anchorId } };
    }
    case "selection.restore": {
      const ids = new Set(action.ids);
      return {
        ...state,
        selection: {
          ids,
          anchorId: action.anchorId !== null && ids.has(action.anchorId) ? action.anchorId : null,
        },
      };
    }
    case "selection.all":
      return { ...state, selection: { ids: new Set(action.ids), anchorId: action.ids.at(0) ?? null } };
    case "detail.open": {
      if (state.detail.tabs.includes(action.id)) {
        return { ...state, detail: { ...state.detail, activeId: action.id, minimized: false } };
      }
      const tabs = [...state.detail.tabs];
      const parentIndex = action.parentId === undefined ? -1 : tabs.indexOf(action.parentId);
      if (parentIndex >= 0) tabs.splice(parentIndex + 1, 0, action.id);
      else tabs.push(action.id);
      return { ...state, detail: { tabs, activeId: action.id, minimized: false } };
    }
    case "detail.activate":
      return { ...state, detail: { ...state.detail, activeId: action.id } };
    case "detail.close":
      return closeDetail(state, action.id);
    case "detail.closeAll":
      return { ...state, detail: { tabs: [], activeId: null, minimized: false } };
    case "detail.minimize":
      return { ...state, detail: { ...state.detail, minimized: action.minimized } };
    case "overlay.activity":
      return { ...state, overlays: { ...state.overlays, activityOpen: action.open } };
    case "overlay.settings":
      return { ...state, overlays: { ...state.overlays, settingsOpen: action.open } };
    case "overlay.review":
      return { ...state, overlays: { ...state.overlays, reviewGalleryId: action.galleryId } };
    case "overlay.exit":
      return { ...state, overlays: { ...state.overlays, exitConfirmOpen: action.open } };
  }
}
