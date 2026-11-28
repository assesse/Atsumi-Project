import { useEffect, useState } from 'react';
import type { MediaReplayChatItem, SoopGiftMetadata } from '../platforms/platformMedia';
import { safeChatImage } from '../platforms/PlatformChatParts';

const MAX_GIFT_VALUE = 0xffffffff;
const isOrdinal = (value: unknown): value is number => typeof value === 'number' && Number.isSafeInteger(value) && value >= 0 && value <= MAX_GIFT_VALUE;

export function validSoopGift(row: MediaReplayChatItem): row is MediaReplayChatItem & { soopGift: SoopGiftMetadata } {
  const gift = row.soopGift;
  return row.kind === 'donation' && !!gift && (gift.kind === 'balloon' || gift.kind === 'videoBalloon')
    && isOrdinal(gift.count) && gift.count > 0 && typeof gift.relay === 'boolean' && typeof gift.isDefault === 'boolean';
}

function balloonAsset(count: number) {
  const step = count < 10 ? 1 : count < 50 ? 2 : count < 100 ? 3 : count < 500 ? 4 : count < 1000 ? 5 : 6;
  return `/assets/soop/gifts/ba_step${step}.png`;
}

/** The original LIVE balloon markup, with locally pinned assets and saved profile actions only. */
export function OriginalSoopGiftMessage({ row, images, showTimes, onProfile }: {
  row: MediaReplayChatItem & { soopGift: SoopGiftMetadata };
  images: ReadonlyMap<string, string>;
  showTimes: boolean;
  onProfile(): void;
}) {
  const gift = row.soopGift;
  const video = gift.kind === 'videoBalloon';
  const localImage = typeof gift.imageAssetId === 'string'
    && /^[a-f0-9]{64}$/.test(gift.imageAssetId) ? images.get(gift.imageAssetId) : undefined;
  const [failedImage, setFailedImage] = useState<string>();
  useEffect(() => setFailedImage(undefined), [localImage]);
  const savedImage = localImage !== failedImage && safeChatImage(localImage) ? localImage : undefined;
  const custom = !video && !gift.isDefault;
  const customImage = custom && !!savedImage;
  const fallbackTitle = custom ? '당시 맞춤 이미지는 저장되지 않아 기본 이미지로 표시합니다.'
    : '당시 이미지가 없거나 읽을 수 없어 앱에 포함된 기준 이미지로 표시합니다.';
  const seconds = Math.max(0, Math.floor((row.timeMs ?? 0) / 1000));
  const author = () => <button type="button" className="name platform-replay-chat-author" onClick={onProfile}>{row.author}</button>;
  return <>
    <div className="chatting-list-item platform-replay-chat-row" data-chat-row-id={row.id} user-type="">
      {showTimes && row.timeMs !== null ? <time className="soop-replay-time">{Math.floor(seconds / 60)}:{String(seconds % 60).padStart(2, '0')}</time> : null}
      <div className="donation-container"><div className="donation-bubble">
        <div className={`ceremony-img${video || customImage ? ' custom-mobile-img' : ''}`}>
          <img src={savedImage ?? (video ? '/assets/soop/gifts/m_video_balloon.png' : balloonAsset(gift.count))} alt={video ? '영상풍선' : custom && !savedImage ? '별풍선 기본 이미지' : '별풍선'} onError={savedImage ? () => setFailedImage(savedImage) : undefined} />
          {!customImage ? <span className="count-box" aria-hidden="true"><em {...{ count: String(gift.count) }}>{gift.count}</em></span> : null}
        </div>
        <div className="info-box">
          {author()}
          <span className="money">{gift.relay ? '중계방 ' : ''}{video ? <>영상풍선 <em>{gift.count}개</em></> : <>별풍선 <em>{gift.count}</em>개</>}</span>
          {!savedImage ? <small className="soop-replay-time" title={fallbackTitle}>{custom ? '기본 이미지' : '기준 이미지'}</small> : null}
        </div>
      </div></div>
    </div>
    {isOrdinal(gift.fanOrder) && gift.fanOrder > 0 ? <div className="chatting-list-item" user-type="fan"><div className="system-message donation-state"><p className="msg">{author()}님이 {gift.fanOrder}번째 팬클럽이 되셨습니다.</p></div></div> : null}
    {gift.topFan === true ? <div className="chatting-list-item" user-type="vip"><div className="system-message donation-state"><p className="msg">{author()}님이 열혈팬이 되셨습니다.</p></div></div> : null}
  </>;
}
