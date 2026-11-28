import type { SoopPersonalconMetadata, SoopAvatarMetadata, SoopChatMetadata } from '../platforms/platformMedia';

function flag(value: SoopChatMetadata | undefined, mask: number): boolean {
  return !!value && Number.isInteger(value.flag1) && value.flag1 >= 0 && value.flag1 <= 0xffffffff && (value.flag1 & mask) !== 0;
}
export function isSoopBroadcaster(value?: SoopChatMetadata): boolean { return value?.role === 'streamer' || flag(value, 4); }
export function isSoopNightbot(value?: SoopChatMetadata): boolean { return !isSoopBroadcaster(value) && (value?.role === 'nightbot' || flag(value, 1 << 26)); }
export function validSoopAvatar(value: SoopAvatarMetadata | null | undefined): value is SoopAvatarMetadata {
  return !!value && value.kind === 'broadcaster' && value.source === 'connectionMetadata' && /^[a-f0-9]{64}$/.test(value.imageAssetId);
}

export function validSoopPersonalcon(value: SoopPersonalconMetadata | null | undefined): value is SoopPersonalconMetadata {
  return !!value && /^[a-f0-9]{64}$/.test(value.imageAssetId) && value.source === 'connectionMetadata'
    && (value.tier === 1 || value.tier === 2 || value.tier === 3)
    && Number.isSafeInteger(value.months) && (value.months === -1 || value.months >= 1 && value.months <= 1200);
}
export function personalconTierLabel(value: SoopPersonalconMetadata): string {
  const label = value.tierNickname;
  return typeof label === 'string' && label.length > 0 && [...label].length <= 80 && new TextEncoder().encode(label).length <= 320
    && !/[\u0000-\u001f\u007f-\u009f\ud800-\udfff]/u.test(label) ? label : `구독 ${value.tier}단계`;
}
