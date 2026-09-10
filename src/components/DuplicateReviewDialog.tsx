import type { DownloadOverlapDecisionRequest, DownloadOverlapMergeRequest, DuplicateReview } from "../api/contracts";
import type { Gallery, GalleryId } from "../core/types";
import type { ThumbnailClient } from "../thumbnail";
import { completedPairReview } from "../state/completedPairReview";
import { DownloadOverlapReviewDialog } from "./DownloadOverlapReviewDialog";

// Stored/global candidates use the same rendering and controls as downloads.
export function DuplicateReviewDialog({ review, ...props }: {
  open: boolean; review?: DuplicateReview; galleries?: ReadonlyMap<GalleryId, Gallery>;
  loading?: boolean; error?: string | null; decisionPending?: boolean; browserFixture?: boolean;
  thumbnailClient?: ThumbnailClient; previewWidth?: number;
  onClose: () => void; onRetry: () => void; onRescan: () => void;
  onDecision: (request: DownloadOverlapDecisionRequest) => void;
  onMergePages?: (request: DownloadOverlapMergeRequest) => void;
}) {
  return <DownloadOverlapReviewDialog {...props} previewWidth={props.previewWidth ?? 320} review={review ? completedPairReview(review) : undefined} completedPair />;
}
