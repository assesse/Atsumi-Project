import { useEffect, useMemo, useRef } from "react";
import { createAutoWatchApi, createAutoWatchLiveApi, type AutoWatchApi, type AutoWatchTarget } from "../../api/autoWatch";
import type { OfficialBrowserApi } from "../../api/officialBrowser";
import { OfficialBrowserPanel } from "./OfficialBrowserPanel";

/** Only lease lifetime differs. Player, chat, settings, screenshots and capture
 * controls are the normal live screen, targeting the existing receiver. */
export function AutoRecordingLiveView({ target, runtime, active = true, privacy, onLeave, api: suppliedApi, liveApi: suppliedLiveApi }: {
  target: AutoWatchTarget; runtime: AutoWatchApi["runtime"]; privacy: boolean; onLeave(): void;
  active?: boolean;
  api?: AutoWatchApi; liveApi?: OfficialBrowserApi;
}) {
  const api = useMemo(() => suppliedApi ?? createAutoWatchApi(runtime), [runtime, suppliedApi]);
  const id = target.watchId!;
  const liveApi = useMemo(() => suppliedLiveApi ?? createAutoWatchLiveApi(runtime, id, api), [runtime, id, api, suppliedLiveApi]);
  const mounted = useRef(false), released = useRef(false), version = useRef(0);
  const retiring = useRef(new Map<string, symbol>());
  useEffect(() => {
    retiring.current.delete(id);
    mounted.current = true; released.current = false;
    return () => {
      mounted.current = false; version.current++;
      if (!released.current) {
        const token = Symbol(id), pending = retiring.current;
        pending.set(id, token);
        queueMicrotask(() => {
          if (pending.get(id) !== token) return;
          pending.delete(id); void api.close(id).catch(() => {});
        });
      }
    };
  }, [api, id]);
  const close = async () => {
    const request = version.current;
    const result = await api.close(id);
    if (!mounted.current || request !== version.current) return;
    if (!result.ok) throw new Error(result.error.message);
    released.current = true; onLeave();
  };
  return <OfficialBrowserPanel key={id} runtime={runtime} active={active} view="live" api={liveApi}
    privacyMode={privacy} connectionLocked onRecordOnly={close} />;
}
