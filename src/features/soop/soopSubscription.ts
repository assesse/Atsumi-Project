import type { SoopSubscriptionMetadata } from '../platforms/platformMedia';

const ID = /^[a-f0-9]{64}$/;
/** Same engineering bounds as the native public91/93 parser, not an entitlement check. */
export function validSoopSubscription(value: unknown): value is SoopSubscriptionMetadata {
  if (!value || typeof value !== 'object') return false;
  const item = value as SoopSubscriptionMetadata;
  if (item.displayTier !== 1 && item.displayTier !== 2) return false;
  if (item.event === 'new') return typeof item.fromVod === 'boolean'
    && typeof item.productMonths === 'number' && [1, 3, 6, 12].includes(item.productMonths) && item.months == null;
  return item.event === 'renewal' && typeof item.months === 'number'
    && Number.isInteger(item.months) && item.months >= 1 && item.months <= 1200
    && item.fromVod == null && item.productMonths == null;
}

export function soopSubscriptionAssetIds(value: unknown): string[] {
  if (!validSoopSubscription(value)) return [];
  return [value.imageAssetId, value.fallbackAssetId]
    .filter((id): id is string => typeof id === 'string' && ID.test(id));
}

export function soopSubscriptionNickname(value: unknown): string | undefined {
  if (typeof value !== 'string' || value.trim().length === 0 || /[\u0000-\u001f\u007f-\u009f]/.test(value)) return undefined;
  const scalars = Array.from(value);
  return scalars.length <= 80 && !scalars.some(char => /^[\ud800-\udfff]$/.test(char))
    && new TextEncoder().encode(value).byteLength <= 320 ? value : undefined;
}
