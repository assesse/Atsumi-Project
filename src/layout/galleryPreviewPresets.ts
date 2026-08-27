import type { CSSProperties } from "react";
import presetData from "../../shared/gallery-preview-presets.json";

export type GalleryPreviewPreset = Readonly<{
  key: string;
  width: number;
  legacyMax?: number;
  maxTagRows: number;
  titlePx: number;
  bodyPx: number;
  tagPx: number;
  footerPx: number;
  titleLinePx: number;
  bodyLinePx: number;
  tagLinePx: number;
  footerLinePx: number;
  chipHeightPx: number;
  paddingPx: number;
  gapPx: number;
}>;

export const GALLERY_PREVIEW_PRESETS = Object.freeze(
  presetData.map((preset) => Object.freeze({ ...preset })),
) as readonly GalleryPreviewPreset[];

export const DEFAULT_GALLERY_PREVIEW_WIDTH = 220;

export function normalizeGalleryPreviewWidth(value: number): number {
  const finite = Number.isFinite(value) ? value : DEFAULT_GALLERY_PREVIEW_WIDTH;
  return (GALLERY_PREVIEW_PRESETS.find((preset) =>
    preset.legacyMax !== undefined && finite <= preset.legacyMax)
    ?? GALLERY_PREVIEW_PRESETS.at(-1)!).width;
}

export function galleryPreviewPreset(value: number): GalleryPreviewPreset {
  const normalized = normalizeGalleryPreviewWidth(value);
  return GALLERY_PREVIEW_PRESETS.find((preset) => preset.width === normalized)
    ?? GALLERY_PREVIEW_PRESETS[2]!;
}

export function galleryPreviewPresetIndex(value: number): number {
  const normalized = normalizeGalleryPreviewWidth(value);
  return Math.max(0, GALLERY_PREVIEW_PRESETS.findIndex((preset) => preset.width === normalized));
}

export function galleryPreviewPresetStyle(preset: GalleryPreviewPreset): CSSProperties {
  return {
    "--preview-width": `${preset.width}px`,
    "--card-title-size": `${preset.titlePx}px`,
    "--card-body-size": `${preset.bodyPx}px`,
    "--card-tag-size": `${preset.tagPx}px`,
    "--card-footer-size": `${preset.footerPx}px`,
    "--card-title-line": `${preset.titleLinePx}px`,
    "--card-body-line": `${preset.bodyLinePx}px`,
    "--card-tag-line": `${preset.tagLinePx}px`,
    "--card-footer-line": `${preset.footerLinePx}px`,
    "--card-chip-height": `${preset.chipHeightPx}px`,
    "--card-padding": `${preset.paddingPx}px`,
    "--card-gap": `${preset.gapPx}px`,
    "--card-tag-max-rows": preset.maxTagRows,
    "--card-tag-max-height": `${preset.chipHeightPx * preset.maxTagRows + preset.gapPx * (preset.maxTagRows - 1)}px`,
  } as CSSProperties;
}
