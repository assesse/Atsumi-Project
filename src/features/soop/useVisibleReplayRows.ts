import { useEffect, useState } from 'react';

/** Asset admission follows the existing scroll viewport; it never changes chat
 * history or video time. Offscreen history cannot evict the visible artwork. */
export function useVisibleReplayRows(scope: string, ids: readonly string[], enabled: boolean, root: HTMLDivElement | null): ReadonlySet<string> | null {
  const key = JSON.stringify([scope, ids]);
  const [visible, setVisible] = useState<{ root: HTMLDivElement | null; key: string; ids: ReadonlySet<string> }>({ root: null, key: '', ids: new Set() });
  const supported = typeof IntersectionObserver !== 'undefined';
  useEffect(() => {
    if (!enabled || !supported || !root) return;
    let alive = true;
    const current = new Set<string>();
    const allowed = new Set(ids);
    const observer = new IntersectionObserver(entries => {
      if (!alive) return;
      for (const entry of entries) {
        const id = (entry.target as HTMLElement).dataset.chatRowId;
        if (!id || !allowed.has(id)) continue;
        if (entry.isIntersecting) current.add(id); else current.delete(id);
      }
      setVisible(previous => previous.root === root && previous.key === key && previous.ids.size === current.size && [...current].every(id => previous.ids.has(id))
        ? previous : { root, key, ids: new Set(current) });
    }, { root, threshold: 0 });
    for (const node of root.querySelectorAll('[data-chat-row-id]')) observer.observe(node);
    return () => { alive = false; observer.disconnect(); };
    // `key` contains the exact ordered ID list; new media ticks alone must not
    // tear down visibility or schedule more local asset reads.
  }, [key, enabled, supported, root]);
  if (!supported) return null;
  return enabled && visible.root === root && visible.key === key ? visible.ids : new Set<string>();
}
