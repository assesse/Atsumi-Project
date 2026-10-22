import { describe, expect, it } from 'vitest';
import { emptyMediaSelection, retainMediaSelection, selectMedia } from './mediaSelection';

const visible = ['a', 'b', 'c', 'd', 'e'];
const plain = { ctrl: false, shift: false };
const ctrl = { ctrl: true, shift: false };
const shift = { ctrl: false, shift: true };

describe('independent platform media selection', () => {
  it('selects one item and clears a repeated solo click and its anchor', () => {
    const selected = selectMedia(emptyMediaSelection(), 'b', visible, plain);
    expect(selected).toEqual({ ids: ['b'], anchorId: 'b' });
    expect(selectMedia(selected, 'b', visible, plain)).toEqual(emptyMediaSelection());
  });
  it('toggles individual Ctrl items and uses the most recent Ctrl item as anchor', () => {
    const first = selectMedia(emptyMediaSelection(), 'a', visible, plain);
    const added = selectMedia(first, 'e', visible, ctrl);
    expect(added).toEqual({ ids: ['a', 'e'], anchorId: 'e' });
    expect(selectMedia(added, 'a', visible, ctrl)).toEqual({ ids: ['e'], anchorId: 'a' });
  });
  it('adds a range without losing noncontiguous selection or moving the anchor', () => {
    const current = { ids: ['a', 'e'], anchorId: 'e' };
    const back = selectMedia(current, 'c', visible, shift);
    expect(back).toEqual({ ids: ['a', 'e', 'c', 'd'], anchorId: 'e' });
    const forward = selectMedia(back, 'd', visible, shift);
    expect(forward).toEqual(back);
    expect(selectMedia(forward, 'b', visible, shift)).toEqual({ ids: ['a', 'e', 'c', 'd', 'b'], anchorId: 'e' });
    expect(current).toEqual({ ids: ['a', 'e'], anchorId: 'e' });
  });
  it('gives Shift precedence over Ctrl so Ctrl+Shift adds the same visible range', () => {
    expect(selectMedia({ ids: ['a', 'e'], anchorId: 'e' }, 'c', visible, { ctrl: true, shift: true }))
      .toEqual({ ids: ['a', 'e', 'c', 'd'], anchorId: 'e' });
  });
  it('replaces a multiple selection with one plain-clicked item', () => {
    expect(selectMedia({ ids: ['a', 'c'], anchorId: 'c' }, 'a', visible, plain)).toEqual({ ids: ['a'], anchorId: 'a' });
  });
  it('prunes removed IDs and their anchor while preserving unchanged state identity', () => {
    const current = { ids: ['a', 'c'], anchorId: 'c' };
    expect(retainMediaSelection(current, visible)).toBe(current);
    expect(retainMediaSelection(current, ['a', 'b'])).toEqual({ ids: ['a'], anchorId: null });
    expect(retainMediaSelection(current, [])).toEqual(emptyMediaSelection());
  });
  it('uses only visible range IDs and falls back to solo selection after the anchor disappears', () => {
    expect(selectMedia({ ids: ['a', 'c'], anchorId: 'a' }, 'e', ['a', 'd', 'e'], shift))
      .toEqual({ ids: ['a', 'd', 'e'], anchorId: 'a' });
    expect(selectMedia({ ids: ['a', 'c'], anchorId: 'c' }, 'd', ['a', 'b', 'd'], shift))
      .toEqual({ ids: ['d'], anchorId: 'd' });
  });
  it('ignores an unknown clicked ID without admitting hidden selection', () => {
    const current = { ids: ['a'], anchorId: 'a' };
    expect(selectMedia(current, 'hidden', visible, plain)).toBe(current);
    expect(selectMedia({ ids: ['a', 'hidden'], anchorId: 'hidden' }, 'hidden', visible, ctrl))
      .toEqual({ ids: ['a'], anchorId: null });
  });
});
