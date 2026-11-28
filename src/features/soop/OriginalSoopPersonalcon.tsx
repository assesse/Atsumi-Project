import { useState } from 'react';
import type { SoopChatMetadata } from '../platforms/platformMedia';
import { safeChatImage } from '../platforms/PlatformChatParts';
import { isSoopBroadcaster, isSoopNightbot, personalconTierLabel, validSoopAvatar, validSoopPersonalcon } from './soopPersonalcon';

/** Capture-time selection and local immutable artwork; never a current profile lookup. */
export function OriginalSoopPersonalcon({ metadata, images }: { metadata?: SoopChatMetadata; images: ReadonlyMap<string, string> }) {
  const broadcaster = isSoopBroadcaster(metadata), nightbot = isSoopNightbot(metadata);
  const value = !nightbot && validSoopPersonalcon(metadata?.personalcon) ? metadata.personalcon : null;
  const avatar = broadcaster && !value && validSoopAvatar(metadata?.avatar) ? metadata.avatar : null;
  const local = value ? images.get(value.imageAssetId) : avatar ? images.get(avatar.imageAssetId) : undefined;
  const [failed, setFailed] = useState<string>();
  const image = safeChatImage(local) && local !== failed ? local : undefined;
  if (nightbot) return <span className="thumb" title="나이트봇"><img src="/assets/soop/thumb_nightbot.svg" alt="" /></span>;
  if (avatar) return <span className="thumb" title={image ? '저장된 스트리머 프로필' : '프로필 기본 이미지 · 저장된 스트리머 이미지를 불러오지 못했습니다.'}>
    <img src={image ?? '/assets/soop/thumb_profile.svg'} alt={image ? '프로필 이미지' : '프로필 기본 이미지'} onError={image ? () => setFailed(image) : undefined} />
  </span>;
  const legacyTier = metadata?.subscriptionTier;
  if (!value && legacyTier !== 1 && legacyTier !== 2 && legacyTier !== 3) return null;
  const title = image ? '저장된 구독 퍼스나콘' : value
    ? '구독 기본 아이콘 · 저장된 채널별 아이콘을 불러오지 못했습니다.'
    : '구독 기본 아이콘 · 당시 채널별 아이콘은 저장되지 않았습니다.';
  const months = value ? value.months : metadata?.accumulatedSubscriptionMonths;
  return <span className="thumb" title={title}>
    <img src={image ?? '/assets/soop/signature-default.svg'} alt={image ? '구독 퍼스나콘' : '구독 기본 퍼스나콘'}
      onError={image ? () => setFailed(image) : undefined} />
    {!broadcaster ? <span className="tier_tip"><em className="name">{value ? personalconTierLabel(value) : `구독 ${legacyTier}단계`}</em>{typeof months === 'number' && months > 0 ? ` · 누적 ${months}개월` : ''}</span> : null}
  </span>;
}
