import type { SoopEmoticonMetadata } from '../platforms/platformMedia';

const ID = /^[a-f0-9]{64}$/;
export function validSoopEmoticon(value: unknown): value is SoopEmoticonMetadata {
  if (!value || typeof value !== 'object') return false;
  const item = value as SoopEmoticonMetadata;
  return (item.kind === 'ogq' || item.kind === 'gem') && typeof item.animated === 'boolean';
}
export function soopEmoticonAssetIds(value: unknown): string[] {
  if (!validSoopEmoticon(value)) return [];
  return [value.staticImageAssetId, value.smallStaticImageAssetId,
    ...(value.animated ? [value.animatedImageAssetId, value.smallAnimatedImageAssetId] : [])]
    .filter((id): id is string => typeof id === 'string' && ID.test(id));
}
export function soopEmoticonDisplayAssetIds(value: unknown, small: boolean, animate: boolean): string[] {
  if (!validSoopEmoticon(value)) return [];
  return [small ? value.smallStaticImageAssetId : value.staticImageAssetId,
    value.animated && animate ? small ? value.smallAnimatedImageAssetId : value.animatedImageAssetId : null]
    .filter((id): id is string => typeof id === 'string' && ID.test(id));
}

// Exact keys in SOOP's original LIVE nicknameRandomColor table. Theme-specific
// colors are supplied by the locally pinned original CSS, never guessed.
const nicknameKeys = ['E12E2E','A90A0A','EB500D','C25111','E07E0A','B26C03','648E0B','119937','158304','047143',
  '0799B9','0B6D82','0C80D3','0C61E1','1915BF','6518C8','AD19E1','DD32E0','DC3585','C40F70'];
export function soopNicknameClass(key: unknown, role: string): string {
  const index = typeof key === 'string' && role !== 'streamer' ? nicknameKeys.indexOf(key) : -1;
  return index < 0 ? '' : `random-color${index + 1}`;
}
