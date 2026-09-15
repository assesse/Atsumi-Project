import { useCommonNavigation } from "../../app/CommonNavigation";
import type { WorkKey } from "./api";

export function CommunityReviewButton({ work, small = false }: { work: WorkKey; small?: boolean }) {
  const navigation = useCommonNavigation();
  if (!navigation) return null;

  return (
    <button
      type="button"
      className={`icon-button community-review-button${small ? " small" : ""}`}
      aria-label="후기 남기기"
      title="후기 남기기"
      onClick={() => navigation.openCommunity(work)}
    >
      <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true" focusable="false">
        <path d="M5 3.5h14a2 2 0 0 1 2 2v11a2 2 0 0 1-2 2H9l-5 3v-3a2 2 0 0 1-1-1.73V5.5a2 2 0 0 1 2-2Z" />
        <path d="M7.5 8.5h9m-9 5h6" />
      </svg>
    </button>
  );
}
