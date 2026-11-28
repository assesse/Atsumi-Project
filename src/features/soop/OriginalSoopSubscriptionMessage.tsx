import { useState } from 'react';
import type { MediaReplayChatItem, SoopSubscriptionMetadata } from '../platforms/platformMedia';
import { safeChatImage } from '../platforms/PlatformChatParts';
import { soopSubscriptionAssetIds, soopSubscriptionNickname } from './soopSubscription';

const GENERIC_IMAGE = '/assets/soop/subscriptions/gudok_1.png';

/** Original LIVE91/93 ceremony DOM. Images and profile information are local replay data. */
export function OriginalSoopSubscriptionMessage({ row, images, showTimes, onProfile }: {
  row: MediaReplayChatItem & { soopSubscription: SoopSubscriptionMetadata };
  images: ReadonlyMap<string, string>;
  showTimes: boolean;
  onProfile(): void;
}) {
  const subscription = row.soopSubscription;
  const allowedIds = soopSubscriptionAssetIds(subscription);
  const localImage = (id: string | null | undefined) => {
    const value = id && allowedIds.includes(id) ? images.get(id) : undefined;
    return safeChatImage(value) ? value : undefined;
  };
  const primary = localImage(subscription.imageAssetId), fallback = localImage(subscription.fallbackAssetId);
  // A different source/result starts a fresh decode chain. Keep the comparison in
  // render so a stale failure never hides the new image for even a single frame.
  const [failures, setFailures] = useState<{ rowId: string; primary?: string; fallback?: string; values: string[] }>({ rowId: '', values: [] });
  const failed = failures.rowId === row.id && failures.primary === primary && failures.fallback === fallback ? failures.values : [];
  const src = [primary, fallback, GENERIC_IMAGE].find((value): value is string => !!value && !failed.includes(value));
  const generic = !primary || src !== primary;
  const defaultName = subscription.displayTier === 2 ? '플러스' : '베이직';
  const nickname = soopSubscriptionNickname(subscription.tierNickname);
  const customName = nickname && nickname !== defaultName ? nickname : undefined;
  const tier = <>{defaultName}{customName ? <> <em>{customName}</em></> : null}</>;
  const newText = `${subscription.fromVod ? 'VOD에서 ' : ''}${customName ? '' : `${defaultName} `}구독하였습니다.`;
  const seconds = Math.max(0, Math.floor((row.timeMs ?? 0) / 1000));
  return <div className="chatting-list-item platform-replay-chat-row" data-chat-row-id={row.id} user-type="">
    {showTimes && row.timeMs !== null ? <time className="soop-replay-time">{Math.floor(seconds / 60)}:{String(seconds % 60).padStart(2, '0')}</time> : null}
    <div className="donation-container subscribe"><div className="donation-bubble">
      <div className="ceremony-img custom-mobile-img">
        {src ? <img src={src} alt={generic ? '구독 기본 이미지' : subscription.event === 'new' ? '구독 이미지' : '연속 구독 이미지'}
          onError={() => setFailures({ rowId: row.id, primary, fallback, values: [...failed, src] })} /> : null}
      </div>
      <div className="info-box">
        <button type="button" className="name platform-replay-chat-author" onClick={onProfile}>{row.author}</button>
        <span className="tier_info">{subscription.event === 'renewal' || customName ? tier : newText}</span>
        {subscription.event === 'renewal' ? <span className="money"><em>{subscription.months}</em>개월째 구독중</span>
          : customName ? <span className="money">{newText}</span> : null}
        {generic ? <small className="soop-replay-time" title="저장된 구독 알림 이미지를 표시하지 못해 기본 이미지로 표시합니다.">{src ? '기본 이미지' : '이미지 없음'}</small> : null}
      </div>
    </div></div>
  </div>;
}
