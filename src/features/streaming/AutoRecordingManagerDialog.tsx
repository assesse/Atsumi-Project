import { useEffect, useRef } from "react";
import { createPortal } from "react-dom";
import type { AutoRecordingApi, AutoRecordingSnapshot } from "../../api/autoRecording";
import { AutoRecordingPanel } from "./AutoRecordingPanel";
import { FluentIcon } from "../../components/FluentIcon";

/** Advanced controls stay reachable without restoring a separate workspace tab. */
export function AutoRecordingManagerDialog({ runtime, privacy, api, onChange, onClose }: {
  runtime: "tauri" | "browser-mock"; privacy: boolean; api: AutoRecordingApi;
  onChange(snapshot: AutoRecordingSnapshot): void; onClose(): void;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  const closeButton = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    const opener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const node = dialog.current;
    if (node && !node.open) {
      if (typeof node.showModal === "function") node.showModal();
      else node.setAttribute("open", "");
    }
    closeButton.current?.focus();
    return () => { if (opener?.isConnected) opener.focus({ preventScroll: true }); };
  }, []);
  return createPortal(<dialog ref={dialog} className="auto-record-manager-dialog" aria-label="자동 녹화 관리"
    data-native-preserve-video="true"
    onKeyDown={event => event.stopPropagation()}
    onCancel={event => { event.preventDefault(); onClose(); }}
    onClose={onClose}>
    <div className="auto-record-manager-surface" data-native-dialog-surface="true">
      <button ref={closeButton} type="button" className="auto-record-manager-close" aria-label="자동 녹화 관리 닫기" onClick={onClose}><FluentIcon glyph="\uE711" /></button>
      <AutoRecordingPanel runtime={runtime} privacy={privacy} api={api} embedded onChange={onChange} />
    </div>
  </dialog>, document.body);
}
