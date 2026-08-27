import { useEffect, useRef } from "react";
import type { AppUpdateState } from "../update/useAppUpdater";

type UpdateDialogProps = {
  open: boolean;
  state: AppUpdateState;
  onLater: () => void;
  onInstall: () => void;
};

const formatDate = (value?: string): string | null => {
  if (!value) return null;
  const parsed = new Date(value);
  if (Number.isNaN(parsed.getTime())) return value;
  return new Intl.DateTimeFormat("ko-KR", { dateStyle: "long" }).format(parsed);
};

const formatBytes = (value: number): string => {
  if (value < 1024) return `${value} B`;
  if (value < 1024 * 1024) return `${(value / 1024).toFixed(1)} KB`;
  return `${(value / (1024 * 1024)).toFixed(1)} MB`;
};

export function UpdateDialog({ open, state, onLater, onInstall }: UpdateDialogProps) {
  const dialog = useRef<HTMLDialogElement>(null);
  const installButton = useRef<HTMLButtonElement>(null);
  const opener = useRef<HTMLElement | null>(null);
  const closingInternally = useRef(false);
  const busy = state.phase === "downloading" || state.phase === "installing";
  const date = formatDate(state.info?.date);
  const percent = state.totalBytes && state.totalBytes > 0
    ? Math.min(100, Math.round((state.downloadedBytes / state.totalBytes) * 100))
    : null;

  useEffect(() => {
    const node = dialog.current;
    if (!node) return;
    const wasOpen = node.open;
    if (open && !wasOpen) {
      opener.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
      node.showModal();
      window.requestAnimationFrame(() => installButton.current?.focus());
    } else if (!open && wasOpen) {
      closingInternally.current = true;
      node.close();
      const target = opener.current;
      opener.current = null;
      window.requestAnimationFrame(() => target?.isConnected && target.focus());
    }
  }, [open]);

  return (
    <dialog
      ref={dialog}
      className="update-dialog"
      aria-labelledby="update-dialog-title"
      onCancel={(event) => {
        event.preventDefault();
        if (!busy) onLater();
      }}
      onClose={() => {
        if (closingInternally.current) {
          closingInternally.current = false;
          return;
        }
        if (!busy) onLater();
      }}
    >
      <div className="update-dialog-body">
        <header className="update-dialog-header">
          <div>
            <span className="eyebrow">APP UPDATE</span>
            <h2 id="update-dialog-title">업데이트가 있습니다</h2>
          </div>
          <button type="button" className="exit-dialog-close" aria-label="나중에 업데이트" title="나중에 업데이트" disabled={busy} onClick={onLater}>×</button>
        </header>

        {state.info ? (
          <>
            <div className="update-version-row" aria-label={`현재 버전 ${state.info.currentVersion}, 새 버전 ${state.info.version}`}>
              <span>현재 v{state.info.currentVersion}</span>
              <span aria-hidden="true">→</span>
              <strong>새 버전 v{state.info.version}</strong>
            </div>
            {date ? <p className="update-release-date">배포일 {date}</p> : null}
            <section className="update-release-notes" aria-labelledby="update-notes-title">
              <strong id="update-notes-title">업데이트 내용</strong>
              <p>{state.info.notes ?? "새 버전의 안정성 개선과 기능 변경이 포함되어 있습니다."}</p>
            </section>
          </>
        ) : null}

        <p className="update-security-note">
          Windows 게시자 인증서는 사용하지 않지만, 업데이트 파일은 Atsumi Next 전용 업데이트 키로 검증한 뒤 설치합니다.
        </p>

        {busy ? (
          <div className="update-progress" role="status" aria-live="polite">
            <div>
              <strong>{state.phase === "installing" ? "설치 중" : "다운로드 중"}</strong>
              <span>
                {state.phase === "installing"
                  ? "완료되면 앱을 다시 시작합니다."
                  : percent !== null
                    ? `${percent}% · ${formatBytes(state.downloadedBytes)} / ${formatBytes(state.totalBytes ?? 0)}`
                    : formatBytes(state.downloadedBytes)}
              </span>
            </div>
            <progress value={state.phase === "installing" ? undefined : percent ?? undefined} max="100" />
          </div>
        ) : null}

        {state.error ? <p className="update-error" role="alert">{state.error}</p> : null}

        <footer className="update-dialog-actions">
          <button type="button" className="text-button" disabled={busy} onClick={onLater}>나중에</button>
          <button ref={installButton} type="button" className="text-button primary" disabled={busy || !state.info} onClick={onInstall}>
            {state.phase === "error" ? "다시 시도" : state.phase === "installing" ? "설치 중" : state.phase === "downloading" ? "다운로드 중" : "다운로드 및 설치"}
          </button>
        </footer>
      </div>
    </dialog>
  );
}
