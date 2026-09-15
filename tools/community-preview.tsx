import { useState } from "react";
import { createRoot } from "react-dom/client";
import { CommonNavigationContext } from "../src/app/CommonNavigation";
import { CommunityWorkspace } from "../src/features/community/CommunityWorkspace";
import type { CommunityApi, Review, WorkKey } from "../src/features/community/api";
import "../src/styles.css";

// UI fixtures only: never sends a request or issues a real identity.
let reviews: Review[] = [
  { id: "fixture-1", source: "hitomi", workId: "10000001", nickname: "작은서재", rating: 5, recommended: true, comment: "이 화면은 실제 후기가 아닌 UI 확인용 예시입니다.\n별점만 간단히 남기거나, 이렇게 짧은 감상을 나눌 수 있어요.", createdAt: "2026-09-14T14:00:00Z", updatedAt: "2026-09-14T14:00:00Z" },
  { id: "fixture-2", source: "danbooru", workId: "10000002", nickname: "하늘빛", rating: 4, recommended: false, comment: "이미지와 개인 다운로드 정보는 게시판에 올라가지 않습니다.", createdAt: "2026-09-14T12:00:00Z", updatedAt: "2026-09-14T12:00:00Z" },
];
const same = (review: Review, key: WorkKey) => review.source === key.source && review.workId === key.workId;
const api: CommunityApi = {
  feed: async (source) => ({ items: reviews.filter((review) => !source || review.source === source), nextCursor: null }),
  work: async (key) => ({ items: reviews.filter((review) => same(review, key)), nextCursor: null }),
  beginWriting: async () => ({ profile: { id: "fixture-member", nickname: "내 작은서재" }, mine: null }),
  save: async (input) => { reviews = [{ ...input, id: `fixture-${Date.now()}`, createdAt: new Date().toISOString(), updatedAt: new Date().toISOString() }, ...reviews]; },
  delete: async () => undefined, report: async () => undefined,
};
function Preview() {
  const [collapsed, setCollapsed] = useState(false);
  return <CommonNavigationContext.Provider value={{ communityOpen: true, openCommunity: () => undefined }}><CommunityWorkspace source="hitomi" collapsed={collapsed} attentionCount={0} onToggleRail={() => setCollapsed((value) => !value)} onSourceChange={() => undefined} onNavigate={() => undefined} onSettings={() => undefined} api={api} /></CommonNavigationContext.Provider>;
}
createRoot(document.getElementById("root")!).render(<Preview />);
