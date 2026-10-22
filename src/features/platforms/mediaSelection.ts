// Independently owned by the platform media library. Mirrors the album-card
// gesture contract without importing Hitomi or CHZZK state, IDs or reducers.
export type MediaSelection = { ids: string[]; anchorId: string | null };
export const emptyMediaSelection = (): MediaSelection => ({ ids: [], anchorId: null });

export function selectMedia(state: MediaSelection, id: string, visibleIds: readonly string[], modifiers: { ctrl: boolean; shift: boolean }): MediaSelection {
  const visible = retainMediaSelection(state, visibleIds);
  if (!visibleIds.includes(id)) return visible;
  const selected = new Set(visible.ids);
  let anchorId = visible.anchorId;
  if (modifiers.shift && anchorId !== null) {
    const start = visibleIds.indexOf(anchorId), end = visibleIds.indexOf(id);
    for (const value of visibleIds.slice(Math.min(start, end), Math.max(start, end) + 1)) selected.add(value);
  } else if (modifiers.ctrl) {
    if (selected.has(id)) selected.delete(id); else selected.add(id);
    anchorId = id;
  } else {
    const repeatedSingle = selected.size === 1 && selected.has(id);
    selected.clear();
    if (repeatedSingle) anchorId = null;
    else { selected.add(id); anchorId = id; }
  }
  return { ids: [...selected], anchorId };
}

export function retainMediaSelection(state: MediaSelection, visibleIds: readonly string[]): MediaSelection {
  const allowed = new Set(visibleIds), ids = state.ids.filter(id => allowed.has(id));
  const anchorId = state.anchorId !== null && allowed.has(state.anchorId) ? state.anchorId : null;
  return ids.length === state.ids.length && anchorId === state.anchorId ? state : { ids, anchorId };
}
