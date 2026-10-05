import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import type { ReplayMessage } from "../../api/replay";

export function ReplayUserCard({ item, anchor, portrait, badges, onClose }: {
  item: ReplayMessage; anchor: HTMLButtonElement; portrait: string | null;
  badges: { src: string | null; label: string }[]; onClose(): void;
}) {
  const card = useRef<HTMLDivElement>(null);
  const [position, setPosition] = useState({ left: 8, top: 8, width: 304, maxHeight: 400 });
  const surface = anchor.getRootNode() as ShadowRoot;
  const storedMonths = item.rich?.subscriptionMonths ?? Number(/^구독 (\d+)개월$/.exec(item.rich?.badges.find(badge => badge.kind === "subscription")?.title ?? "")?.[1] ?? NaN);
  const months = Number.isInteger(storedMonths) && storedMonths >= 0 && storedMonths <= 1200 ? storedMonths : null;
  const subscription = months === null ? null : `${months >= 12 ? `${Math.floor(months / 12)}년 ` : ""}${months % 12 || months === 0 ? `${months % 12}개월 ` : ""}구독 중`;
  const follow = /^(\d{4})-(\d{2})-(\d{2})$/.exec(item.rich?.followingSince ?? "");
  useLayoutEffect(() => {
    const place = () => {
      const host = surface.host.getBoundingClientRect(), target = anchor.getBoundingClientRect();
      const width = Math.max(0, Math.min(304, host.width - 16));
      const height = card.current?.getBoundingClientRect().height || 170;
      const preferred = target.bottom - host.top + 6;
      setPosition({ width, maxHeight: Math.max(80, host.height - 16),
        left: Math.max(8, Math.min(target.left - host.left, host.width - width - 8)),
        top: Math.max(8, Math.min(preferred + height > host.height - 8 ? target.top - host.top - height - 6 : preferred, host.height - height - 8)) });
    };
    place(); window.addEventListener("resize", place);
    card.current?.querySelector<HTMLButtonElement>("button")?.focus({ preventScroll: true });
    return () => window.removeEventListener("resize", place);
  }, [anchor, surface]);
  useEffect(() => {
    const outside = (event: PointerEvent) => { if (!event.composedPath().includes(anchor) && !event.composedPath().includes(card.current!)) onClose(); };
    const escape = (event: KeyboardEvent) => { if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); onClose(); anchor.focus({ preventScroll: true }); } };
    const scroll = (event: Event) => { if (event.target instanceof Element && event.target.classList.contains("recording-replay-chat-scroll")) onClose(); };
    document.addEventListener("pointerdown", outside); surface.addEventListener("keydown", escape as EventListener, true); surface.addEventListener("scroll", scroll, true);
    return () => { document.removeEventListener("pointerdown", outside); surface.removeEventListener("keydown", escape as EventListener, true); surface.removeEventListener("scroll", scroll, true); };
  }, [anchor, surface, onClose]);
  return createPortal(<div ref={card} className="recording-replay-user-card theme_dark" role="dialog" aria-label={`${item.sender} 사용자 정보`} style={position}>
    <button type="button" className="recording-replay-user-close" aria-label="사용자 정보 닫기" onClick={() => { onClose(); anchor.focus({ preventScroll: true }); }}>×</button>
    <div className="recording-replay-user-heading">
      {portrait ? <img className="recording-replay-user-avatar" src={portrait} alt="" referrerPolicy="no-referrer" /> : <span className="recording-replay-user-avatar" aria-hidden="true">{Array.from(item.sender)[0] || "?"}</span>}
      <div><strong>{item.sender}</strong><div className="recording-replay-user-badges">{badges.map((badge, index) => badge.src ? <img key={index} src={badge.src} alt={badge.label} title={badge.label} referrerPolicy="no-referrer" /> : <small key={index}>{badge.label}</small>)}</div></div>
    </div>
    <div className="recording-replay-user-history">
      {subscription && <p><span aria-hidden="true">★</span>{subscription}</p>}
      {follow && <p><span aria-hidden="true">♥</span>{`${Number(follow[1])}년 ${Number(follow[2])}월 ${Number(follow[3])}일부터 팔로우`}</p>}
      {!subscription && !follow && <p className="recording-replay-user-unknown">저장된 구독·팔로우 정보 없음</p>}
      <small>녹화 당시 정보</small>
    </div>
  </div>, surface);
}

const noticeLabels = { donation: "후원", video_donation: "영상 후원", mission: "미션", subscription: "구독" };
const missionLabels: Record<string, string> = { PENDING: "대기", OPEN: "진행 중", ACCEPTED: "수락", REJECTED: "거절", COMPLETED: "종료", SUCCESS: "성공", FAILURE: "실패", FAILED: "실패", CANCELED: "취소", EXPIRED: "시간 만료" };
export function ReplayNoticeHeading({ item }: { item: ReplayMessage }) {
  const notice = item.rich?.notice;
  if (!notice || !(notice.kind in noticeLabels)) return null;
  return <div className="recording-replay-notice-heading"><strong>{noticeLabels[notice.kind]}</strong>
    {notice.amount != null && <span>{notice.amount.toLocaleString("ko-KR")} 치즈</span>}
    {notice.kind === "subscription" && notice.months != null && <span>{notice.months}개월</span>}
    {notice.kind === "mission" && notice.status && missionLabels[notice.status] && <span className="recording-replay-mission-status">{missionLabels[notice.status]}</span>}
  </div>;
}
