import { useMemo } from "react";
import type { Gallery, UiState } from "../core/types";
import type { PopularityRanks } from "../api/downloadPopularity";
import { visibleGalleries } from "../state/selectors";

/** Navigation/selection is urgent and must not re-run a whole library query. */
export function useVisibleGalleries(ui: UiState, galleries: Gallery[], popularity: PopularityRanks): Gallery[] {
  const { view, downloadsFilter, downloadsSort, exploreSort } = ui;
  const search = ui.search[view];
  return useMemo(() => view === "explore" ? galleries : visibleGalleries({
    view, downloadsFilter, downloadsSort, exploreSort,
    search: { explore: search, downloads: search, "auto-find": search },
  }, galleries, popularity), [view, downloadsFilter, downloadsSort, exploreSort,
    search.committed, search.languages, galleries, popularity]);
}
