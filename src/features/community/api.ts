import { invoke, isTauri } from "@tauri-apps/api/core";

export type CommunitySource = "hitomi" | "danbooru";
export type WorkKey = { source: CommunitySource; workId: string };
export type Cursor = { createdAt: string; id: string };
export type Review = WorkKey & { id: string; nickname: string; rating: number; recommended: boolean; comment: string; createdAt: string; updatedAt: string };
export type Mine = Pick<Review, "id" | "rating" | "recommended" | "comment"> & { hidden: boolean };
export type Summary = WorkKey & { reviewCount: number; averageRating: number | null; recommendationCount: number };
export type ReviewPage = { items: Review[]; nextCursor: Cursor | null; summary?: Summary };
export type Writer = { profile: { id: string; nickname: string }; mine: Mine | null };
export type ReviewInput = WorkKey & Pick<Review, "nickname" | "rating" | "recommended" | "comment">;
export interface CommunityApi {
  feed(source: CommunitySource | null, cursor?: Cursor | null): Promise<ReviewPage>;
  work(key: WorkKey, cursor?: Cursor | null): Promise<ReviewPage>;
  beginWriting(key: WorkKey): Promise<Writer>;
  save(input: ReviewInput): Promise<void>;
  delete(key: WorkKey): Promise<void>;
  report(reviewId: string, reason: string): Promise<void>;
}

const projectUrl = "https://yfpgshvflnawmrimyfzo.supabase.co";
const publishableKey = "sb_publishable_oRRDkcQgUf1LMzM6pe9blQ_0AGjcS7z";
export const communityError = (error: unknown): string => typeof error === "string" ? error : error instanceof Error ? error.message : "커뮤니티 요청을 완료하지 못했습니다.";
export const validWorkId = (value: string) => /^[1-9][0-9]{0,19}$/.test(value);

// Public browser preview only. Production uses native networking with a fixed
// endpoint, so existing CSP stays closed. No personal token in JS/localStorage.
async function publicRpc<T>(name: string, body: unknown): Promise<T> {
  const response = await fetch(`${projectUrl}/rest/v1/rpc/community_v1_${name}`, {
    method: "POST", headers: { apikey: publishableKey, "Content-Type": "application/json" },
    body: JSON.stringify(body), signal: AbortSignal.timeout(20_000), credentials: "omit",
  });
  if (!response.ok) throw new Error(response.status === 429 ? "요청이 많습니다. 잠시 후 다시 시도해 주세요." : "후기를 불러오지 못했습니다. 서버 연결을 확인해 주세요.");
  return await response.json() as T;
}
const write = <T,>(request: unknown): Promise<T> => {
  if (!isTauri()) return Promise.reject(new Error("작성자 키를 안전하게 보관할 수 있는 Windows용 Atsumi 앱에서 작성해 주세요. 브라우저에서는 열람만 가능합니다."));
  return invoke<T>("community_write", { request });
};
export const communityApi: CommunityApi = {
  feed: (source, cursor = null) => isTauri()
    ? invoke("community_read", { request: { kind: "feed", source, cursor } })
    : publicRpc("feed", { p_source: source, p_cursor: cursor }),
  work: async (key, cursor = null) => {
    if (isTauri()) return invoke("community_read", { request: { kind: "work", ...key, cursor } });
    const [page, summaries] = await Promise.all([
      publicRpc<ReviewPage>("reviews", { p_source: key.source, p_work_id: key.workId, p_cursor: cursor }),
      publicRpc<Summary[]>("summaries", { p_works: [key] }),
    ]);
    return { ...page, summary: summaries[0] };
  },
  beginWriting: (key) => write({ kind: "beginWriting", ...key }),
  save: (input) => write({ kind: "save", ...input }),
  delete: (key) => write({ kind: "delete", ...key }),
  report: (reviewId, reason) => write({ kind: "report", reviewId, reason }),
};
