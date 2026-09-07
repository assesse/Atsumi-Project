import type { Gallery, GalleryId } from "../core/types";

const UNKNOWN_ARTIST_BUCKET = "\u0000artist-unknown";
const UNKNOWN_ARTIST_LABELS: ReadonlySet<string> = new Set([
  "정보 불러오는 중",
  "알 수 없는 작가",
  "작가 정보 없음",
]);

const normalizedArtist = (artist: string): string => artist
  .normalize("NFKC")
  .trim()
  .replace(/\s+/gu, " ")
  .toLocaleLowerCase();

const artistBucketKey = (gallery: Gallery): string => {
  const displayArtist = normalizedArtist(gallery.artist);
  if (!displayArtist || UNKNOWN_ARTIST_LABELS.has(displayArtist)) return UNKNOWN_ARTIST_BUCKET;
  return displayArtist;
};

const randomIndex = (length: number, random: () => number): number => {
  const sample = random();
  const bounded = Number.isFinite(sample) ? Math.max(0, Math.min(sample, 0.9999999999999999)) : 0;
  return Math.floor(bounded * length);
};

/**
 * Chooses a completed local album without letting prolific artists dominate.
 *
 * The first draw chooses a normalized artist bucket uniformly and the second
 * chooses one of that artist's eligible albums uniformly. Albums whose artist
 * metadata is blank share one explicit "unknown artist" bucket rather than
 * each gaining an artist-sized share of the draw.
 */
export function pickArtistBalancedCompletedDownload(
  galleries: readonly Gallery[],
  excludedGalleryIds: ReadonlySet<GalleryId>,
  random: () => number = Math.random,
): Gallery | undefined {
  const artistBuckets = new Map<string, Gallery[]>();
  for (const gallery of galleries) {
    if (gallery.download?.state !== "completed" || excludedGalleryIds.has(gallery.id)) continue;
    const key = artistBucketKey(gallery);
    const bucket = artistBuckets.get(key);
    if (bucket) bucket.push(gallery);
    else artistBuckets.set(key, [gallery]);
  }

  const buckets = [...artistBuckets.values()];
  if (!buckets.length) return undefined;
  const bucket = buckets[randomIndex(buckets.length, random)]!;
  return bucket[randomIndex(bucket.length, random)];
}
