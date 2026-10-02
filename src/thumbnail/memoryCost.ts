import type { ThumbnailAsset } from "./client";

/** A display URL can keep both encoded bytes and a decoded RGBA surface alive.
 * Compressed file size alone is not a safe WebView memory budget. This is a
 * conservative accounting estimate, not a measurement of Chromium's heap. */
export const thumbnailMemoryCost = (asset: ThumbnailAsset): number => {
  if (asset.kind === "missing") return 1024;
  const pixels = asset.kind === "image"
    ? asset.width * asset.height
    : asset.sheetWidth * asset.sheetHeight; // A sprite retains the whole sheet.
  const encoded = asset.kind === "image" && asset.byteLength !== undefined
    ? asset.byteLength
    : Math.max(16 * 1024, Math.ceil(pixels / 2));
  return Math.min(Number.MAX_SAFE_INTEGER, Math.ceil(pixels * 4) + encoded);
};
