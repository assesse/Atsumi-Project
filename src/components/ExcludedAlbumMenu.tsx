import { useEffect, useState, type ComponentProps } from "react";
import { backend } from "../api/backend";
import type { ExplorationExclusionContext } from "../api/contracts";
import type { GalleryId } from "../core/types";
import { CardContextMenu } from "./CardContextMenu";

export function ExcludedAlbumMenu({ galleryId, pending = false, onInspect, onRestore, onOpenRetained, ...menu }: Pick<ComponentProps<typeof CardContextMenu>, "anchor" | "close" | "label"> & {
  galleryId: GalleryId; pending?: boolean;
  onInspect(id: GalleryId, context: ExplorationExclusionContext): void;
  onRestore(id: GalleryId): void;
  onOpenRetained(id: GalleryId): void;
}) {
  const [context, setContext] = useState<ExplorationExclusionContext | null>(null);
  const [error, setError] = useState(false);
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    let disposed = false;
    setContext(null); setError(false);
    void backend.explorationExclusionContext(galleryId).then((result) => {
      if (disposed) return;
      if (result.ok) setContext(result.data); else setError(true);
    }).catch(() => { if (!disposed) setError(true); });
    return () => { disposed = true; };
  }, [galleryId, attempt]);
  const retained = context?.retainedGallery;
  return <CardContextMenu {...menu} items={[
    { id: "exclusion-evidence", label: !context && !error ? "제외 정보 확인 중…" : "제외 근거 보기", disabled: !context || pending,
      action: () => { if (context) onInspect(galleryId, context); } },
    { id: "exclusion-restore", label: pending ? "복원 중…" : "제외 해제·복원", disabled: !context || pending || (!context.quarantined && !context.reasons.length),
      action: () => onRestore(galleryId) },
    { id: "exclusion-retained", label: retained ? `보존된 판본 열기 · #${retained.galleryId}` : "보존된 판본 연결 없음", disabled: !retained || pending,
      action: () => { if (retained) onOpenRetained(retained.galleryId); } },
    ...(error ? [{ id: "retry", label: "정보를 불러오지 못했습니다 · 다시 시도", keepOpen: true, action: () => setAttempt((value) => value + 1) }] : []),
  ]} />;
}
