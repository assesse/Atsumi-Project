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

function CommunityFeed({ source, order, expanded, hidden, revision, api, privacyMode, onExpand }: {
  source: CommunitySource; order: FeedOrder; expanded: boolean; hidden: boolean; revision: number;
  api: CommunityApi; privacyMode: boolean; onExpand: () => void;
}) {
  const label = feedOrders.find(option => option.value === order)!.label;
  const [page, setPage] = useState<FeedPage>({ items: [], nextCursor: null });
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const loadSequence = useRef(0);
  const pending = useRef(false);
  const load = useCallback(async (cursor: FeedCursor | null = null) => {
    const sequence = ++loadSequence.current;
    pending.current = true;
    setLoading(true); setError(null);
    try {
      const result = await api.feed(source, cursor, order);
      if (sequence !== loadSequence.current) return;
      const items = result.items.filter((item) => item.source === source);
      setPage((previous) => ({ ...result, items: cursor ? [...previous.items, ...items.filter((item) => !previous.items.some((old) => order === "latest" ? old.id === item.id : old.workId === item.workId))] : items }));
    } catch (error) { if (sequence === loadSequence.current) setError(communityError(error)); }
    finally { if (sequence === loadSequence.current) { pending.current = false; setLoading(false); } }
  }, [api, source, order]);
  useEffect(() => { setPage({ items: [], nextCursor: null }); setNotice(null); void load(); return () => { ++loadSequence.current; }; }, [load, revision]);
  const gridClass = expanded ? "community-review-grid" : "community-review-row";
  return <section className="community-feed" aria-label={`${label} 목록`} aria-busy={loading} hidden={hidden}>
    <header className="community-feed-heading"><h2>{label}</h2><div className="community-feed-actions">
      <button type="button" className="icon-button" title="새로고침" aria-label={`${label} 새로고침`} aria-busy={loading} disabled={loading} onClick={() => { if (!pending.current) void load(); }}>
        {loading ? <span className="spinner catalog-refresh-spinner" aria-hidden="true" /> : <FluentIcon glyph="\uE72C" />}
      </button>
      {!expanded ? <button type="button" className="text-button" aria-label={`${label} 더 보기`} onClick={onExpand}>더 보기</button> : null}
    </div></header>
    {error ? <p className="community-error" role="alert">{error}</p> : null}
    {notice ? <p className="community-notice" role="status">{notice}</p> : null}
    {loading && !page.items.length ? <div className={`community-loading ${gridClass}`} role="status" aria-label={`${label} 불러오는 중`}>{[1, 2, 3, 4].map(key => <div key={key} />)}</div> : null}
    {!loading && !error && !page.items.length ? <div className="community-empty">아직 등록된 후기가 없습니다.</div> : null}
    <div className={gridClass}>{page.items.filter(review => review.source === source).slice(0, expanded ? undefined : 6).map(review =>
      <ReviewCard key={review.id} review={review} privacyMode={privacyMode} onReport={async reason => { await api.report(review.id, reason); setNotice("신고가 접수되었습니다. 운영자가 확인합니다."); }} />
    )}</div>
    {expanded && page.nextCursor ? <button type="button" className="text-button community-more" disabled={loading} onClick={() => { if (!pending.current) void load(page.nextCursor); }}>{loading ? "불러오는 중…" : "후기 더 보기"}</button> : null}
  </section>;
}

export function CommunityWorkspace({ source, collapsed, autoFindCount = 0, attentionCount, onToggleRail, onSourceChange, onNavigate, onSettings, onOpenPersonalLibrary, privacyMode = false, privacyModePending = false, onPrivacyModeToggle, initialReview = null, api = communityApi }: Props) {
  const [dedicatedOrder, setDedicatedOrder] = useState<FeedOrder | null>(null);
  const [revision, setRevision] = useState(0);
  const [mineOpen, setMineOpen] = useState(!!initialReview);
  const [writingShortcut, setWritingShortcut] = useState(initialReview);
  const workspace = useRef<HTMLElement>(null);
  useEffect(() => { setDedicatedOrder(null); }, [source]);
  const openFeed = (order: FeedOrder | null) => { setDedicatedOrder(order); if (workspace.current) workspace.current.scrollTop = 0; };
  return <div className={`app-shell community-shell${collapsed ? " sidebar-collapsed" : ""}`}>
    <SideRail source={source} view={workspaceRegistry[source].navigation[0].view} collapsed={collapsed} autoFindCount={autoFindCount} attentionCount={attentionCount} onNavigate={onNavigate} onSourceChange={onSourceChange} onToggle={onToggleRail} onOpenPersonalLibrary={onOpenPersonalLibrary} onSettings={onSettings} privacyMode={privacyMode} privacyModePending={privacyModePending} onPrivacyModeToggle={onPrivacyModeToggle} />
    <main className="community-workspace" ref={workspace}>
      <header className="community-header"><div><span className="eyebrow">ATSUMI COMMUNITY</span><h1>커뮤니티</h1><p>작품과 함께 보는 한마디.</p></div><div className="community-header-actions">
        <button className="icon-button" type="button" aria-label="내 후기" title="내 후기" onClick={() => { setWritingShortcut(null); setMineOpen(true); }}><FluentIcon glyph="\uE77B" /></button>
      </div></header>
      <div className="community-content">
        {dedicatedOrder ? <div className="community-dedicated-toolbar">
          <button type="button" className="text-button" aria-label="돌아가기" onClick={() => openFeed(null)}><FluentIcon glyph="\uE72B" /> 돌아가기</button>
          <select aria-label="후기 목록" value={dedicatedOrder} onChange={event => openFeed(event.target.value as FeedOrder)}>
            {feedOrders.map(option => <option key={option.value} value={option.value}>{option.label}</option>)}
          </select>
        </div> : null}
        {feedOrders.filter(option => option.value !== "worst" || dedicatedOrder === "worst").map(option => <CommunityFeed
          key={`${source}:${option.value}`} source={source} order={option.value} expanded={dedicatedOrder === option.value}
          hidden={dedicatedOrder !== null && dedicatedOrder !== option.value} revision={revision}
          api={api} privacyMode={privacyMode} onExpand={() => openFeed(option.value)} />)}
        <details className="community-help"><summary>익명 키와 공개 범위 안내</summary><p>공개되는 정보는 사이트·작품번호·닉네임·후기·별점·추천 여부입니다. 다운로드 내역, 기기 식별 정보, 이미지, 파일 경로는 커뮤니티 서버에 전송하지 않습니다. 미리보기는 원본 사이트에서 불러옵니다. Supabase는 서비스 운영에 필요한 접속 정보(IP 등)를 처리할 수 있습니다.</p><p>작성자 키는 Windows 사용자 계정으로 암호화해 앱 설정·캐시와 별도로 보관합니다. 앱 초기화는 키와 서버 후기를 지우지 않습니다. Windows 재설치 또는 보관 파일을 직접 삭제하면 이전 후기의 수정·삭제 권한을 잃을 수 있습니다. 현재 다른 PC로 옮기거나 분실한 키를 복구하는 기능은 없습니다.</p><p>닉네임은 중복될 수 있으며 본인 인증을 의미하지 않습니다. 작품마다 이 앱의 작성자 키로 후기 한 개를 남길 수 있습니다. 개인정보·불법 콘텐츠·도배는 게시하지 마세요.</p></details>
      </div>
    </main>
    {mineOpen ? <MyReviewsDialog key={source} source={source} api={api} privacyMode={privacyMode} initialWork={writingShortcut?.source === source ? writingShortcut : null} onClose={() => { setMineOpen(false); setWritingShortcut(null); }} onChanged={() => setRevision(value => value + 1)} /> : null}
  </div>;
}
