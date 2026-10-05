import { useCallback, useEffect, useRef, useState } from "react";
import type { ContentSource, WorkspaceViewId } from "../../app/workspaceRegistry";
import { workspaceRegistry } from "../../app/workspaceRegistry";
import { SideRail } from "../../components/SideRail";
import { FluentIcon } from "../../components/FluentIcon";
import { communityApi, communityError, type CommunityApi, type CommunitySource, type FeedCursor, type FeedOrder, type FeedPage, type WorkKey } from "./api";
import { ReviewCard } from "./ReviewCard";
import { MyReviewsDialog } from "./MyReviewsDialog";
import "./CommunityWorkspace.css";

type Props = {
  source: CommunitySource; collapsed: boolean; autoFindCount?: number; attentionCount: number;
  onToggleRail: () => void; onSourceChange: (source: ContentSource) => void;
  onNavigate: (view: WorkspaceViewId) => void; onSettings: () => void;
  privacyMode?: boolean; privacyModePending?: boolean; onPrivacyModeToggle?: () => void;
  initialReview?: WorkKey | null; api?: CommunityApi;
  onOpenPersonalLibrary?: () => void;
};

const feedOrders: { value: FeedOrder; label: string; description: string }[] = [
  { value: "popular", label: "인기 후기", description: "별점·후기 수를 함께 반영 · 작품당 최근 후기" },
  { value: "latest", label: "최신 후기", description: "최근 작성된 후기부터" },
  { value: "worst", label: "최악의 작품", description: "작품 평균 별점이 낮은 순 · 작품당 최근 후기" },
];

export function CommunityWorkspace({ source, collapsed, autoFindCount = 0, attentionCount, onToggleRail, onSourceChange, onNavigate, onSettings, onOpenPersonalLibrary, privacyMode = false, privacyModePending = false, onPrivacyModeToggle, initialReview = null, api = communityApi }: Props) {
  const [order, setOrder] = useState<FeedOrder>("popular");
  const [page, setPage] = useState<FeedPage>({ items: [], nextCursor: null });
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [mineOpen, setMineOpen] = useState(!!initialReview);
  const [writingShortcut, setWritingShortcut] = useState(initialReview);
  const loadSequence = useRef(0);
  const load = useCallback(async (cursor: FeedCursor | null = null) => {
    const sequence = ++loadSequence.current;
    setLoading(true); setError(null);
    try {
      const result = await api.feed(source, cursor, order);
      if (sequence !== loadSequence.current) return;
      const items = result.items.filter((item) => item.source === source);
      setPage((previous) => ({ ...result, items: cursor ? [...previous.items, ...items.filter((item) => !previous.items.some((old) => order === "latest" ? old.id === item.id : old.workId === item.workId))] : items }));
    } catch (error) { if (sequence === loadSequence.current) setError(communityError(error)); }
    finally { if (sequence === loadSequence.current) setLoading(false); }
  }, [api, source, order]);
  useEffect(() => { setPage({ items: [], nextCursor: null }); setNotice(null); void load(); return () => { ++loadSequence.current; }; }, [load]);
  return <div className={`app-shell community-shell${collapsed ? " sidebar-collapsed" : ""}`}>
    <SideRail source={source} view={workspaceRegistry[source].navigation[0].view} collapsed={collapsed} autoFindCount={autoFindCount} attentionCount={attentionCount} sourceLabel={`${workspaceRegistry[source].label} · Community`} onNavigate={onNavigate} onSourceChange={onSourceChange} onToggle={onToggleRail} onOpenPersonalLibrary={onOpenPersonalLibrary} />
    <main className="community-workspace">
      <header className="community-header"><div><span className="eyebrow">ATSUMI COMMUNITY</span><h1>커뮤니티</h1><p>작품과 함께 보는 한마디.</p></div><div className="community-header-actions">
        {onPrivacyModeToggle ? <button type="button" className="icon-button" aria-label="프라이버시 모드" aria-pressed={privacyMode} disabled={privacyModePending} onClick={onPrivacyModeToggle}><FluentIcon glyph={privacyMode ? "\uED1A" : "\uE890"} /></button> : null}
        <button type="button" className="text-button" onClick={onSettings}>설정</button>
      </div></header>
      <div className="community-content">
        <div className="community-toolbar" data-tour="community-toolbar">
          <span className="community-source-label">{workspaceRegistry[source].label} 후기</span>
          <button className="text-button primary" type="button" onClick={() => { setWritingShortcut(null); setMineOpen(true); }}>내 후기 보기</button>
        </div>
        <p className="community-privacy">열람은 인증 없이 · 코멘트는 앨범에서 바로 작성</p>
        <section className="community-feed" aria-label="공개 후기 목록" aria-busy={loading}>
          <header className="community-feed-heading"><div className="community-feed-orders" role="group" aria-label="후기 정렬">{feedOrders.map(option => <button key={option.value} type="button" aria-pressed={order === option.value} title={option.value === "popular" ? "평균 3점인 후기 5개를 기준값으로 더해, 후기가 적은 작품의 순위를 보정합니다." : option.description} onClick={() => setOrder(option.value)}>{option.label}</button>)}</div><button type="button" className="text-button community-feed-refresh" aria-busy={loading} disabled={loading} onClick={() => void load()}>{loading ? <><span className="spinner catalog-refresh-spinner" aria-hidden="true" /> 불러오는 중…</> : "새로고침"}</button></header>
          <p className="community-feed-description">{feedOrders.find(option => option.value === order)!.description}</p>
          {error ? <p className="community-error" role="alert">{error}</p> : null}
          {notice ? <p className="community-notice" role="status">{notice}</p> : null}
          {loading && !page.items.length ? <div className="community-loading community-review-grid" role="status" aria-label="후기 불러오는 중">{[1, 2, 3, 4].map((key) => <div key={key} />)}</div> : null}
          {!loading && !error && !page.items.length ? <div className="community-empty"><strong>아직 등록된 후기가 없습니다.</strong><p>앨범 상세보기나 페이지 프리뷰에서 코멘트를 남겨보세요.</p></div> : null}
          <div className="community-review-grid">{page.items.filter(review => review.source === source).map((review) => <ReviewCard key={review.id} review={review} privacyMode={privacyMode} onReport={async (reason) => { await api.report(review.id, reason); setNotice("신고가 접수되었습니다. 운영자가 확인합니다."); }} />)}</div>
          {page.nextCursor ? <button type="button" className="text-button community-more" disabled={loading} onClick={() => void load(page.nextCursor)}>{loading ? "불러오는 중…" : "후기 더 보기"}</button> : null}
        </section>
        <details className="community-help"><summary>익명 키와 공개 범위 안내</summary><p>공개되는 정보는 사이트·작품번호·닉네임·후기·별점·추천 여부입니다. 다운로드 내역, 기기 식별 정보, 이미지, 파일 경로는 커뮤니티 서버에 전송하지 않습니다. 미리보기는 원본 사이트에서 불러옵니다. Supabase는 서비스 운영에 필요한 접속 정보(IP 등)를 처리할 수 있습니다.</p><p>작성자 키는 Windows 사용자 계정으로 암호화해 앱 설정·캐시와 별도로 보관합니다. 앱 초기화는 키와 서버 후기를 지우지 않습니다. Windows 재설치 또는 보관 파일을 직접 삭제하면 이전 후기의 수정·삭제 권한을 잃을 수 있습니다. 현재 다른 PC로 옮기거나 분실한 키를 복구하는 기능은 없습니다.</p><p>닉네임은 중복될 수 있으며 본인 인증을 의미하지 않습니다. 작품마다 이 앱의 작성자 키로 후기 한 개를 남길 수 있습니다. 개인정보·불법 콘텐츠·도배는 게시하지 마세요.</p></details>
      </div>
    </main>
    {mineOpen ? <MyReviewsDialog key={source} source={source} api={api} privacyMode={privacyMode} initialWork={writingShortcut?.source === source ? writingShortcut : null} onClose={() => { setMineOpen(false); setWritingShortcut(null); }} onChanged={() => void load()} /> : null}
  </div>;
}
