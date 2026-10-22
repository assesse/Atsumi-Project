import { useEffect, useState } from "react";
import type { ApiError } from "../../api/contracts";
import "./RecordingLoadStatus.css";

/** A busy response is not a failed request. Keep the API code until presentation. */
export function RecordingLoadStatus({ error, hasSnapshot, subject = "녹화 목록" }: { error: ApiError | null; hasSnapshot: boolean; subject?: string }) {
  const loading = error?.code === "BROWSER_INITIALIZING" || (!error && !hasSnapshot);
  const [visible, setVisible] = useState(false);
  const [slow, setSlow] = useState(false);
  useEffect(() => {
    setVisible(false);
    setSlow(false);
    if (!loading) return;
    const show = setTimeout(() => setVisible(true), 250);
    const delayed = setTimeout(() => setSlow(true), 20_000);
    return () => { clearTimeout(show); clearTimeout(delayed); };
  }, [loading]);

  if (loading) return visible ? <RecordingNotice tone="loading"
    title={slow ? `${subject} 준비가 지연되고 있습니다` : `${subject} 준비 중`}
    detail="다른 탭은 계속 사용할 수 있습니다. 준비가 끝나면 자동으로 표시합니다." /> : null;
  if (!error) return null;
  const failed = error.code === "BROWSER_UNAVAILABLE" || !error.retryable;
  return <RecordingNotice tone={failed ? "error" : "warning"}
    title={failed ? `${subject}을 불러오지 못했습니다` : `${subject === "녹화 목록" ? "녹화 상태" : subject} 확인 실패 · 자동 재확인 중`}
    detail={`${hasSnapshot ? "마지막으로 확인한 목록입니다. " : "아직 목록을 불러오지 못했습니다. "}${error.message}`} />;
}

export function RecordingNotice({ tone, title, detail }: {
  tone: "loading" | "warning" | "error" | "info";
  title: string;
  detail?: string;
}) {
  return <div className={`recording-notice is-${tone}`} role={tone === "error" || tone === "warning" ? "alert" : "status"}>
    <span className="recording-notice-icon" aria-hidden="true">{tone === "loading" ? "" : tone === "error" ? "×" : tone === "warning" ? "!" : "i"}</span>
    <strong>{title}</strong>
    {detail ? <span className="recording-notice-help" tabIndex={0} aria-label={`상세 안내: ${detail}`}>
      <span aria-hidden="true">ⓘ</span><span className="recording-notice-detail" aria-hidden="true">{detail}</span>
    </span> : null}
  </div>;
}
