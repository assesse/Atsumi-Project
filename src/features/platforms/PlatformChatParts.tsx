import { useEffect, useRef, useState } from 'react';
import type { MediaChatAsset, MediaChatPart, SoopGiftMetadata, SoopMissionMetadata, SoopEmoticonMetadata, SoopSubscriptionMetadata, SoopChatMetadata, SoopRowDisplay } from './platformMedia';
import { soopEmoticonDisplayAssetIds } from '../soop/soopEmoticon';
import { soopSubscriptionAssetIds } from '../soop/soopSubscription';
import { soopMissionAssetIds } from '../soop/soopMission';
import { isSoopBroadcaster, isSoopNightbot, validSoopAvatar, validSoopPersonalcon } from '../soop/soopPersonalcon';
import './PlatformChatParts.css';

const ID = /^[a-f0-9]{64}$/;
const DATA = /^data:image\/(?:png|jpeg|gif|webp);base64,[A-Za-z0-9+/]+=*$/;
const MAX_RAM = 8 * 1024 * 1024;
export function safeChatImage(value: unknown): value is string {
  return typeof value === 'string' && value.length <= 1_400_000 && DATA.test(value);
}
/** Local opaque-ID reads only. Two workers, bounded cache, no request on privacy/inactive.
 * Live pending entries are scheduled fairly without holding IPC slots while waiting.
 * Completed replay misses never cause a network recovery request. */
type AssetRow = { parts?: MediaChatPart[]; kind?: string; soop?: SoopChatMetadata; soopGift?: SoopGiftMetadata; soopMission?: SoopMissionMetadata; soopEmoticon?: SoopEmoticonMetadata; soopSubscription?: SoopSubscriptionMetadata; soopDisplay?: SoopRowDisplay };
export function useChatAssets(scope: string, rows: ReadonlyArray<AssetRow>, enabled: boolean,
  load: ((scope: string, id: string) => Promise<MediaChatAsset | string | null>) | undefined, live = false, includeSoopAssets = false,
  soopEmoticonDisplay: { small: boolean; animate: boolean; rowSmall?: (row: AssetRow) => boolean } = { small: false, animate: true }) {
  const [images, setImages] = useState<ReadonlyMap<string, string>>(new Map());
  const cache = useRef({ scope: '', bytes: 0, images: new Map<string, string>(), wanted: new Set<string>(), attempts: new Map<string, { started: number; due: number; count: number; done: boolean; loaded: boolean }>() });
  const inFlight = useRef(0), drainLatest = useRef<() => void>(() => {});
  const ids = [...new Set(rows.flatMap(row => {
    const emojiIds = (row.parts ?? []).slice(0, 128)
      .flatMap(part => part.type === 'emoji' && ID.test(part.assetId) ? [part.assetId] : []);
    const gift = row.soopGift;
    if (includeSoopAssets) emojiIds.push(...soopMissionAssetIds({kind:row.kind??'',soopMission:row.soopMission}));
    if (includeSoopAssets && row.kind === 'donation' && (gift?.kind === 'balloon' || gift?.kind === 'videoBalloon') && typeof gift.isDefault === 'boolean'
      && Number.isSafeInteger(gift.count) && gift.count > 0 && gift.count <= 0xffffffff && typeof gift.relay === 'boolean'
      && typeof gift.imageAssetId === 'string' && ID.test(gift.imageAssetId)) emojiIds.push(gift.imageAssetId);
    // Read only the selected size and its static fallback. Reading all four
    // captured variants would exhaust the256-ID window before120 chat rows.
    if (includeSoopAssets && row.kind === 'chat') emojiIds.push(...soopEmoticonDisplayAssetIds(row.soopEmoticon, soopEmoticonDisplay.rowSmall?.(row) ?? soopEmoticonDisplay.small, soopEmoticonDisplay.animate));
    if (includeSoopAssets && row.kind === 'membership') emojiIds.push(...soopSubscriptionAssetIds(row.soopSubscription));
    if (includeSoopAssets && row.kind === 'chat' && !row.soop?.messageType && !isSoopNightbot(row.soop)) {
      if (validSoopPersonalcon(row.soop?.personalcon)) emojiIds.push(row.soop.personalcon.imageAssetId);
      else if (isSoopBroadcaster(row.soop) && validSoopAvatar(row.soop?.avatar)) emojiIds.push(row.soop.avatar.imageAssetId);
    }
    return emojiIds;
  }))].slice(-256).join(',');
  useEffect(() => {
    if (cache.current.scope !== scope) { cache.current = { scope, bytes: 0, images: new Map(), wanted: new Set(), attempts: new Map() }; setImages(new Map()); }
    if (!enabled || !load || !scope) return;
    const current = cache.current;
    let stopped = false;
    const wanted = ids.split(',').filter(Boolean), busy = new Set<string>();
    let timer: ReturnType<typeof setTimeout> | undefined;
    for (const id of wanted) {
      const prior = current.attempts.get(id);
      const cached = current.images.get(id);
      if (cached !== undefined) { current.images.delete(id); current.images.set(id, cached); }
      // A size toggle/history change may return to an image evicted from RAM.
      // Re-read a known successful local asset once on re-entry, never loop on
      // eviction inside the same wanted set or retry a completed missing asset.
      if (!prior || (prior.loaded && !current.images.has(id) && !current.wanted.has(id)))
        current.attempts.set(id, { started: Date.now(), due: 0, count: 0, done: current.images.has(id), loaded: current.images.has(id) });
    }
    current.wanted = new Set(wanted);
    for (const id of current.attempts.keys()) if (current.attempts.size > 512 && !wanted.includes(id)) current.attempts.delete(id);
    const request = async (id: string) => {
      inFlight.current++; busy.add(id);
      const attempt = current.attempts.get(id)!;
      let result: MediaChatAsset | string | null = null;
      try { result = await load(scope, id); } catch { result = live ? { state: 'pending', dataUrl: null } : null; }
      if (!stopped) {
        const value = typeof result === 'string' ? result : result?.state === 'ready' ? result.dataUrl : null;
        attempt.count++;
        attempt.done = !(live && typeof result === 'object' && result?.state === 'pending' && Date.now() - attempt.started < 360_000);
        attempt.due = Date.now() + ([1000, 2000, 5000][attempt.count - 1] ?? 10_000);
        if (safeChatImage(value)) {
          attempt.loaded = true;
          current.images.set(id, value); current.bytes += value.length;
          while (current.bytes > MAX_RAM || current.images.size > 256) { const first = current.images.keys().next().value!; current.bytes -= current.images.get(first)!.length; current.images.delete(first); }
          setImages(new Map(current.images));
        }
      }
      busy.delete(id); inFlight.current--; drainLatest.current();
    };
    const drain = () => {
      if (timer !== undefined) clearTimeout(timer);
      if (stopped) return;
      const now = Date.now();
      const pending = wanted.filter(id => !busy.has(id) && !current.attempts.get(id)!.done).sort((a, b) => current.attempts.get(a)!.due - current.attempts.get(b)!.due);
      for (const id of pending) {
        const attempt = current.attempts.get(id)!;
        if (now - attempt.started >= 360_000) { attempt.done = true; continue; }
        if (inFlight.current >= 2) break;
        if (attempt.due > now) { timer = setTimeout(drain, attempt.due - now); break; }
        void request(id);
      }
    };
    drainLatest.current = drain;
    drainLatest.current();
    return () => { stopped = true; drainLatest.current = () => {}; if (timer !== undefined) clearTimeout(timer); };
  }, [scope, ids, enabled, load, live]);
  return cache.current.scope === scope ? images : new Map<string, string>();
}
function Emoji({ src, alt }: { src?: string; alt: string }) {
  const [failed, setFailed] = useState(false);
  useEffect(() => setFailed(false), [src]);
  return src && !failed && safeChatImage(src)
    ? <img className="platform-chat-emoji" src={src} alt={alt || '[이모티콘]'} title={alt || '이모티콘'} onError={() => setFailed(true)} draggable={false} />
    : <span className="platform-chat-emoji-fallback" title="이미지를 불러오지 못하면 문자로 표시합니다.">{alt || '[이모티콘]'}</span>;
}
export function PlatformChatParts({ text, parts, images }: { text: string; parts?: MediaChatPart[]; images: ReadonlyMap<string, string> }) {
  if (!parts?.length) return <>{text}</>;
  return <>{parts.slice(0, 128).map((part, index) => part.type === 'text'
    ? <span key={index}>{part.text}</span>
    : part.type === 'emoji' ? <Emoji key={index} src={ID.test(part.assetId) ? images.get(part.assetId) : undefined} alt={part.alt} /> : null)}</>;
}
