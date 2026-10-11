// Independently owned by the recording library. Mirrors the album-card gesture
// contract, without importing Hitomi state, IDs, reducers, or components.
export type RecordingSelection = { ids: string[]; anchorId: string | null };
export const emptyRecordingSelection = (): RecordingSelection => ({ ids: [], anchorId: null });

export function selectRecording(state: RecordingSelection, id: string, visibleIds: string[], modifiers: { ctrl: boolean; shift: boolean }): RecordingSelection {
  if (!visibleIds.includes(id)) return state;
  const selected = new Set(state.ids);
  let anchorId = state.anchorId;
  if (modifiers.shift && anchorId !== null) {
    const start = visibleIds.indexOf(anchorId), end = visibleIds.indexOf(id);
    if (start >= 0 && end >= 0) {
      for (const value of visibleIds.slice(Math.min(start, end), Math.max(start, end) + 1)) selected.add(value);
    }
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

export function retainRecordingSelection(state: RecordingSelection, visibleIds: string[]): RecordingSelection {
  const allowed = new Set(visibleIds), ids = state.ids.filter(id => allowed.has(id));
  const anchorId = state.anchorId !== null && allowed.has(state.anchorId) ? state.anchorId : null;
  return ids.length === state.ids.length && anchorId === state.anchorId ? state : { ids, anchorId };
}
