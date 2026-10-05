import { retryableDownloadStates, type Gallery } from "../core/types";
import { downloadStatus } from "../state/downloadStatus";
import { usePersonalLibrary } from "../features/personalLibrary/PersonalLibraryProvider";
import { targetFor, targetKey } from "../features/personalLibrary/api";
import type { CardMenuItem } from "./CardContextMenu";

export function useAlbumMenuItems(gallery: Gallery | undefined, actions: {
  open(): void;
  background?(): void;
  queue?(): void;
  folder?(): void;
  artifact?(): void;
  review?(): void;
  internalReview?(): void;
  status?(): void;
  exclude?(): void;
  excludeLabel?: string;
  excludeDisabled?: boolean;
  pending?: boolean;
}): CardMenuItem[] {
  const library = usePersonalLibrary();
  if (!gallery) return [];
  const download = gallery.download;
  const saved = library?.index.has(targetKey(targetFor(gallery, 0)));
  return [
    { id: "open", label: "상세 열기", action: actions.open },
    ...(actions.background ? [{ id: "background", label: "백그라운드 탭으로 열기", action: actions.background }] : []),
    ...(actions.folder && download?.state === "completed" ? [{ id: "folder", label: "저장 폴더 열기", action: actions.folder }] : []),
    ...(actions.artifact && download?.state === "completed" ? [{ id: "artifact", label: "외부 뷰어로 열기", action: actions.artifact }] : []),
    ...(actions.queue ? [{ id: "download", label: !download ? "다운로드" : retryableDownloadStates.has(download.state) ? "다운로드 재시도" : `다운로드 · ${downloadStatus[download.state].label}`,
      disabled: actions.pending || Boolean(download && !retryableDownloadStates.has(download.state)), action: actions.queue }] : []),
    ...(library ? [{ id: "bookmark", label: `앨범 즐겨찾기 ${saved ? "해제" : "저장"}`, disabled: library.pending || !library.summary, action: () => { void library.save(gallery, 0, !saved); } }] : []),
    ...(actions.review ? [{ id: "review", label: "중복 판본 검토", separator: true, action: actions.review }] : []),
    ...(actions.internalReview ? [{ id: "internal-review", label: "내부 중복 검토", action: actions.internalReview }] : []),
    ...(actions.status && download ? [{ id: "status", label: "작업 상태 보기", action: actions.status }] : []),
    ...(actions.exclude ? [{ id: "exclude", label: actions.excludeLabel ?? "앞으로 탐색에서 제외", shortcut: "Delete", separator: true, danger: true, disabled: actions.excludeDisabled || actions.pending, action: actions.exclude }] : []),
  ];
}
