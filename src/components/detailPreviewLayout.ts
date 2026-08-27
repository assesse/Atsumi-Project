export const DETAIL_ORIENTATION_SAMPLE_SIZE = 8;

export type DetailPreviewOrientation = "landscape" | "portrait" | "mixed";

export type DetailPreviewSample = Readonly<{
  width?: number;
  height?: number;
}>;

export type DetailPreviewLayout = Readonly<{
  columns: 2 | 3;
  orientation: DetailPreviewOrientation;
}>;

const isValidSample = (sample: DetailPreviewSample): sample is Required<Pick<DetailPreviewSample, "width" | "height">> =>
  Number.isFinite(sample.width)
  && Number.isFinite(sample.height)
  && (sample.width ?? 0) > 0
  && (sample.height ?? 0) > 0;

/**
 * Chooses a regular, keyboard-order-preserving grid. Near-square pages do not
 * bias the decision, and too little evidence stays with the safe 3-column grid.
 */
export function detailPreviewLayout(samples: readonly DetailPreviewSample[]): DetailPreviewLayout {
  const valid = samples.filter(isValidSample);
  if (valid.length < 3) return { columns: 3, orientation: "mixed" };

  let landscape = 0;
  let portrait = 0;
  for (const sample of valid) {
    if (sample.width / sample.height >= 1.15) landscape += 1;
    else if (sample.height / sample.width >= 1.15) portrait += 1;
  }
  if (landscape / valid.length >= 0.6) return { columns: 2, orientation: "landscape" };
  if (portrait / valid.length >= 0.6) return { columns: 3, orientation: "portrait" };
  return { columns: 3, orientation: "mixed" };
}
