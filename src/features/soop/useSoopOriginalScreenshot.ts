import { useEffect, useRef, useState } from 'react';
import type { SoopOriginalApi } from '../../api/soopOriginal';

type Context = { api: SoopOriginalApi; token: string | null; enabled: boolean };
type Preview = { id: string; url: string; undoing: boolean; error: string; undoDeadline: number };
const message = (value: unknown) => typeof value === 'string' ? value : value instanceof Error ? value.message : '스크린샷을 저장하지 못했습니다.';

/** No DOM capture or video request: the original native receiver freezes its frame. */
export function useSoopOriginalScreenshot(context: Context) {
  const owner = useRef(context); owner.current = context;
  const epoch = useRef(0), flight = useRef(false), previewRef = useRef<Preview | null>(null);
  const [busy, setBusy] = useState(false), [error, setError] = useState(''), [preview, setPreview] = useState<Preview | null>(null);
  const clear = (expectedId?: string) => { if (expectedId && previewRef.current?.id !== expectedId) return; previewRef.current = null; setPreview(null); };
  useEffect(() => {
    ++epoch.current; flight.current = false; setBusy(false); setError(''); clear();
    return () => { ++epoch.current; previewRef.current = null; };
  }, [context.api, context.token, context.enabled]);
  const current = (ticket: Context, version: number) => epoch.current === version && owner.current.enabled && ticket.api === owner.current.api && ticket.token === owner.current.token;
  const save = async () => {
    const ticket = owner.current, version = epoch.current;
    if (!ticket.enabled || !ticket.token || flight.current || previewRef.current?.undoing) return;
    const undoDeadline = performance.now() + 45_000;
    flight.current = true; setBusy(true); setError(''); clear();
    try {
      const saved = await ticket.api.screenshot(ticket.token, crypto.randomUUID());
      if (!current(ticket, version)) return;
      const next = { id: saved.item.id, url: saved.preview, undoing: false, error: '', undoDeadline };
      previewRef.current = next; setPreview(next);
    } catch (cause) { if (current(ticket, version)) setError(message(cause)); }
    finally { if (current(ticket, version)) { flight.current = false; setBusy(false); } }
  };
  const undo = async () => {
    const ticket = owner.current, version = epoch.current, shown = previewRef.current;
    if (!ticket.enabled || !ticket.token || !shown || shown.undoing || shown.error || performance.now() >= shown.undoDeadline) return;
    const next = { ...shown, undoing: true }; previewRef.current = next; setPreview(next);
    try { await ticket.api.undoScreenshot(ticket.token, shown.id); if (current(ticket, version) && previewRef.current === next) clear(shown.id); }
    catch { if (current(ticket, version) && previewRef.current === next) { const failed = { ...shown, error: '취소를 확인하지 못했습니다. 스크린샷 목록에서 확인해 주세요.' }; previewRef.current = failed; setPreview(failed); setError(failed.error); } }
  };
  return { busy: context.enabled && busy, error: context.enabled ? error : '', preview: context.enabled ? preview : null, save, undo, dismiss: clear, clearError: () => { setError(''); if (previewRef.current?.error) clear(); } };
}
