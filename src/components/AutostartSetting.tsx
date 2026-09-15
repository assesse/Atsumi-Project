import { useCallback, useEffect, useId, useRef, useState } from "react";
import { getAutostartStatus, setAutostartEnabled, type AutostartStatus } from "../api/autostart";

type AutostartFailure = { message: string; retryEnabled?: boolean };

export function AutostartSetting({ active }: { active: boolean }) {
  const descriptionId = useId();
  const [status, setStatus] = useState<AutostartStatus | null>(null);
  const [reading, setReading] = useState(false);
  const [changing, setChanging] = useState(false);
  const [failure, setFailure] = useState<AutostartFailure | null>(null);
  const request = useRef({ active: false, mounted: false, epoch: 0, readId: 0, changing: false, refreshAfterChange: false });

  const refresh = useCallback(async () => {
    const control = request.current;
    if (!control.active || !control.mounted) return;
    if (control.changing) {
      control.refreshAfterChange = true;
      return;
    }
    const epoch = control.epoch;
    const readId = ++control.readId;
    const current = () => control.mounted && control.active && control.epoch === epoch
      && control.readId === readId && !control.changing;
    setReading(true);
    try {
      const result = await getAutostartStatus();
      if (!current()) return;
      if (result.ok) {
        setStatus(result.data);
        setFailure(null);
      } else {
        setFailure({ message: `자동 실행 상태를 확인하지 못했습니다. ${result.error.message}` });
      }
    } catch {
      if (current()) setFailure({ message: "자동 실행 상태를 확인하지 못했습니다. 잠시 후 다시 시도해 주세요." });
    } finally {
      if (current()) setReading(false);
    }
  }, []);

  useEffect(() => {
    const control = request.current;
    control.mounted = true;
    control.active = active;
    control.epoch += 1;
    control.readId += 1;
    if (active) void refresh();
    const onFocus = () => { void refresh(); };
    if (active) window.addEventListener("focus", onFocus);
    return () => {
      control.active = false;
      control.mounted = false;
      control.epoch += 1;
      control.readId += 1;
      window.removeEventListener("focus", onFocus);
    };
  }, [active, refresh]);

  const change = async (enabled: boolean) => {
    const control = request.current;
    if (!control.active || !status?.supported || control.changing) return;
    const epoch = control.epoch;
    control.changing = true;
    control.readId += 1;
    setChanging(true);
    setReading(false);
    setFailure(null);
    try {
      const result = await setAutostartEnabled(enabled);
      if (!control.mounted || !control.active || control.epoch !== epoch) return;
      if (result.ok) setStatus(result.data);
      else setFailure({ message: `자동 실행 설정을 변경하지 못했습니다. ${result.error.message}`, retryEnabled: enabled });
    } catch {
      if (control.mounted && control.active && control.epoch === epoch) {
        setFailure({ message: "자동 실행 설정을 변경하지 못했습니다. 잠시 후 다시 시도해 주세요.", retryEnabled: enabled });
      }
    } finally {
      control.changing = false;
      if (control.mounted) setChanging(false);
      // A reopened panel or a focus refresh must read the final OS state, not an older response.
      if (control.refreshAfterChange || control.epoch !== epoch) {
        control.refreshAfterChange = false;
        void refresh();
      }
    }
  };

  if (!active) return null;
  const busy = reading || changing;
  const stateLabel = changing ? "변경 중…" : reading ? "확인 중…"
    : !status ? "확인 필요" : !status.supported ? "지원되지 않음"
      : status.disabledByWindows ? "등록됨 · Windows에서 중지"
        : status.needsRepair ? "연결 확인 필요" : status.enabled ? "사용 중" : "사용 안 함";

  return (
    <div className="setting-row" aria-busy={busy}>
      <div>
        <strong>Windows 로그인 시 자동 실행</strong>
        <span id={descriptionId}>켜면 다음 Windows 로그인부터 Atsumi가 실행됩니다. 저장 버튼과 별개로 즉시 적용되며, 기본으로 켜지지 않습니다.</span>
        {status?.supported && status.launchMode === "development" && <span>현재 작업 폴더의 개발 실행기를 사용합니다.</span>}
        {status?.supported && status.launchMode === "installed" && <span>현재 앱의 실행 파일을 사용합니다.</span>}
        {status && !status.supported && <span>Windows 데스크톱 앱에서만 설정할 수 있습니다. 브라우저에서는 변경할 수 없습니다.</span>}
        {status?.needsRepair && <span>이전 앱 위치로 등록되어 있습니다. 현재 앱에 다시 연결하거나 자동 실행을 끌 수 있습니다.</span>}
        {status?.disabledByWindows && <span>Windows 시작 앱 설정에서 사용 안 함으로 지정되어 있습니다. Windows 설정 → 앱 → 시작 프로그램에서 Atsumi를 켜 주세요.</span>}
        {status?.supported && status.needsRepair && (
          <button type="button" className="text-button" disabled={busy} onClick={() => { void change(true); }}>현재 앱에 다시 연결</button>
        )}
        {failure && (
          <>
            <span className="setting-validation-error" role="alert">{failure.message}</span>
            <button type="button" className="text-button" disabled={busy} onClick={() => {
              if (failure.retryEnabled === undefined) void refresh();
              else void change(failure.retryEnabled);
            }}>다시 시도</button>
          </>
        )}
      </div>
      <label className="setting-checkbox">
        <input
          type="checkbox"
          role="switch"
          aria-label="Windows 로그인 시 자동 실행"
          aria-describedby={descriptionId}
          checked={status?.enabled ?? false}
          disabled={!status?.supported || busy}
          onChange={(event) => { void change(event.target.checked); }}
        />
        <span aria-live="polite">{stateLabel}</span>
      </label>
    </div>
  );
}
