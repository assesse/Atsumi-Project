import { useEffect, useRef, useState } from "react";
import type { PinterestAccountApi, PinterestAccountSnapshot } from "./accountApi";
import "./PinterestAccountControls.css";

const empty: PinterestAccountSnapshot = { status: "unknown", busy: false, open: false, generation: 0 };

/** Account proof stays native; this view receives no cookie, name or token. */
export function PinterestAccountControls({ api, active, enabled, onSnapshot }: {
  api: PinterestAccountApi;
  active: boolean;
  enabled: boolean;
  onSnapshot: (snapshot: PinterestAccountSnapshot) => void;
}) {
  const [snapshot, setSnapshot] = useState(empty);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const callback = useRef(onSnapshot);
  callback.current = onSnapshot;
  const alive = useRef(true), request = useRef(0), inFlight = useRef(false), actionPending = useRef(false);
  useEffect(() => { alive.current = true; return () => { alive.current = false; request.current++; }; }, []);
  const accept = (value: PinterestAccountSnapshot) => {
    setSnapshot(value);
    setError(value.error ?? null);
    callback.current(value);
  };
  useEffect(() => {
    if (!active || !enabled) return;
    let disposed = false, timer: ReturnType<typeof setTimeout> | undefined;
    const refresh = async () => {
      if (inFlight.current || actionPending.current) { timer = setTimeout(refresh, 1500); return; }
      const serial = ++request.current;
      let nextDelay = 30000;
      try {
        inFlight.current = true;
        const value = await api.status();
        if (!disposed && alive.current && serial === request.current) {
          accept(value);
          nextDelay = value.open || value.busy ? 1500 : 30000;
        }
      } catch {
        if (!disposed && alive.current && serial === request.current) setError("로그인 상태를 확인하지 못했습니다.");
      } finally {
        inFlight.current = false;
        if (!disposed) timer = setTimeout(refresh, nextDelay);
      }
    };
    void refresh();
    return () => { disposed = true; if (timer) clearTimeout(timer); };
  }, [active, api, enabled, snapshot.open, snapshot.busy]);
  const open = async (action: "login" | "logout") => {
    if (actionPending.current || snapshot.busy || !enabled) return;
    const serial = ++request.current;
    actionPending.current = true;
    setPending(true); setError(null);
    try {
      const value = await api.open(action);
      if (alive.current && serial === request.current) accept(value);
    } catch {
      if (alive.current && serial === request.current) setError(action === "login" ? "로그인 창을 열지 못했습니다." : "로그아웃을 완료하지 못했습니다.");
    } finally { actionPending.current = false; if (alive.current) setPending(false); }
  };
  const busy = pending || snapshot.busy;
  const label = busy ? "확인 중" : snapshot.status === "signedIn" ? "로그인됨" : snapshot.status === "signedOut" ? "로그아웃됨" : "확인 필요";
  return <div className="pinterest-account" aria-label="Pinterest 계정">
    <span role="status" className={snapshot.status === "signedIn" && !busy ? "is-connected" : undefined}>{busy ? <span className="spinner" /> : null}{label}</span>
    <button type="button" className="secondary-button" disabled={!enabled || busy || snapshot.status === "signedIn"} onClick={() => void open("login")}>로그인</button>
    <button type="button" className="secondary-button" disabled={!enabled || busy || snapshot.status !== "signedIn"} onClick={() => void open("logout")}>로그아웃</button>
    {error ? <span className="pinterest-account-error" role="alert">{error}</span> : null}
  </div>;
}
