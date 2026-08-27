import { useEffect, useRef } from "react";
import { hasActiveWork, type AppActiveWorkSnapshot } from "../api/contracts";

type ExitConfirmDialogProps = {
  open: boolean;
  snapshot: AppActiveWorkSnapshot | null;
  statusError: boolean;
  actionPending: boolean;
  forceQuitArmed: boolean;
  onClose: () => void;
  onMinimizeToTray: () => void;
  onQuit: () => void;
};

const progress = (label: string, completed: number, total: number): string | null =>
  total > 0 ? `${label} ${completed}/${total}` : null;

function ActiveWorkStatus({ snapshot }: { snapshot: AppActiveWorkSnapshot }) {
  if (!hasActiveWork(snapshot)) {
    return <p className="exit-work-empty">진행 중인 작업 없음</p>;
  }

  return (
    <>
      <strong className="exit-work-heading">진행 중인 작업</strong>
      <ul className="exit-work-list">
        {snapshot.downloads.activeCount > 0
          ? <li>다운로드 {snapshot.downloads.activeCount}개</li>
          : null}
        {snapshot.autoFind
          ? (
            <li>
              {["Auto Find", progress("작가", snapshot.autoFind.completedFavorites, snapshot.autoFind.totalFavorites), `후보 ${snapshot.autoFind.candidatesFound}개`]
                .filter(Boolean).join(" · ")}
            </li>
          )
          : null}
        {snapshot.duplicateScan
          ? (
            <li>
              {["작품 중복 검사", progress("아티팩트", snapshot.duplicateScan.hashedArtifacts, snapshot.duplicateScan.totalArtifacts), progress("비교", snapshot.duplicateScan.comparedPairs, snapshot.duplicateScan.totalPairs), `후보 ${snapshot.duplicateScan.candidatesFound}개`]
                .filter(Boolean).join(" · ")}
            </li>
          )
          : null}
        {snapshot.internalDuplicateScan
          ? (
            <li>
              {["내부 중복 검사", progress("앨범", snapshot.internalDuplicateScan.scannedArtifacts, snapshot.internalDuplicateScan.totalArtifacts), `제외 ${snapshot.internalDuplicateScan.skippedArtifacts}개`, `검토 행 ${snapshot.internalDuplicateScan.groupsFound}개`]
                .filter(Boolean).join(" · ")}
            </li>
          )
          : null}
      </ul>
      <p className="exit-work-notice">
        종료하면 위 작업을 안전하게 취소한 뒤 앱을 닫습니다.<br />
        완료되지 않은 검사는 다음 실행에서 다시 시작해야 할 수 있습니다.
      </p>
    </>
  );
}

export function ExitConfirmDialog({ open, snapshot, statusError, onClose, onMinimizeToTray, onQuit, actionPending, forceQuitArmed }: ExitConfirmDialogProps) {
  const dialog = useRef<HTMLDialogElement>(null);
  const trayButton = useRef<HTMLButtonElement>(null);
  const opener = useRef<HTMLElement | null>(null);
  const closingInternally = useRef(false);
  const activeWork = snapshot ? hasActiveWork(snapshot) : false;
  const quitLabel = statusError
    ? forceQuitArmed ? "상태 확인 없이 종료" : "다시 확인"
    : activeWork ? "작업을 중단하고 종료" : "종료";

  useEffect(() => {
    const node = dialog.current;
    if (!node) return;
    const wasOpen = node.open;
    if (open && !wasOpen) {
      opener.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
      node.showModal();
      window.requestAnimationFrame(() => trayButton.current?.focus());
    } else if (!open && wasOpen) {
      closingInternally.current = true;
      node.close();
      const target = opener.current;
      opener.current = null;
      window.requestAnimationFrame(() => {
        if (target?.isConnected) target.focus();
        else document.querySelector<HTMLElement>(".view-header input")?.focus();
      });
    }
  }, [open]);

  return (
    <dialog
      className="exit-dialog"
      ref={dialog}
      aria-labelledby="exit-dialog-title"
      onCancel={(event) => {
        event.preventDefault();
        onClose();
      }}
      onClose={() => {
        if (closingInternally.current) {
          closingInternally.current = false;
          return;
        }
        onClose();
      }}
    >
      <div className="exit-dialog-body">
        <div className="exit-dialog-header">
          <h2 id="exit-dialog-title">앱을 닫을까요?</h2>
          <button type="button" className="exit-dialog-close" aria-label="종료 취소" title="종료 취소" disabled={actionPending} onClick={onClose}>×</button>
        </div>
        <div
          className={`exit-work-status${statusError ? " is-error" : activeWork ? " is-working" : ""}`}
          role="status"
          aria-live="polite"
        >
          {statusError
            ? (
              <p>
                작업 상태를 확인할 수 없습니다.
                {forceQuitArmed ? " 상태 확인 없이 종료하려면 다시 선택해 주세요." : " 다시 확인하거나 트레이로 보내 주세요."}
              </p>
            )
            : snapshot
              ? <ActiveWorkStatus snapshot={snapshot} />
              : <p>작업 상태 확인 중</p>}
        </div>
        <div className="exit-dialog-actions">
          <button ref={trayButton} type="button" className="exit-choice primary-choice" disabled={actionPending} onClick={onMinimizeToTray}>
            트레이로 보내기
          </button>
          <button type="button" className="exit-choice quit-choice" title={activeWork ? "진행 중인 작업을 안전하게 중단하고 종료" : "Atsumi 완전히 종료"} disabled={actionPending || (snapshot === null && !statusError)} onClick={onQuit}>
            {quitLabel}
          </button>
        </div>
      </div>
    </dialog>
  );
}
