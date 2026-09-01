export type PagePreviewDimension = Readonly<{
  width?: number;
  height?: number;
}>;

export type PagePreviewViewport = Readonly<{
  width: number;
  height: number;
}>;

export type PagePreviewFrame = Readonly<{
  dialogWidth: number;
  dialogHeight: number;
  mediaWidth: number;
  mediaHeight: number;
  aspectRatio: string;
  orientation: "portrait" | "landscape" | "square";
}>;

export type PagePreviewOrientation = PagePreviewFrame["orientation"];

const VIEWPORT_MARGIN = 24;
const DIALOG_CHROME_HEIGHT = 126;
const DIALOG_BORDER_SIZE = 2;
const MIN_DIALOG_WIDTH = 320;
const FALLBACK_WIDTH = 2;
const FALLBACK_HEIGHT = 3;

const positive = (value: number | undefined): value is number =>
  typeof value === "number" && Number.isFinite(value) && value > 0;

export const validPagePreviewDimension = (
  dimension: PagePreviewDimension | undefined,
): dimension is Readonly<{ width: number; height: number }> =>
  positive(dimension?.width) && positive(dimension?.height);

export const pagePreviewAspect = (
  dimension: PagePreviewDimension | undefined,
): number => validPagePreviewDimension(dimension)
  ? dimension.width / dimension.height
  : FALLBACK_WIDTH / FALLBACK_HEIGHT;

export const pagePreviewOrientation = (
  dimension: PagePreviewDimension | undefined,
): PagePreviewOrientation | undefined => {
  if (!validPagePreviewDimension(dimension)) return undefined;
  const aspect = pagePreviewAspect(dimension);
  return aspect > 1.05 ? "landscape" : aspect < 0.95 ? "portrait" : "square";
};

export const pagePreviewSpreadDimension = (
  first: PagePreviewDimension | undefined,
  second: PagePreviewDimension | undefined,
): PagePreviewDimension | undefined => {
  if (!validPagePreviewDimension(first) || !validPagePreviewDimension(second)) return undefined;
  return {
    width: pagePreviewAspect(first) + pagePreviewAspect(second),
    height: 1,
  };
};

const boundedViewport = (value: number): number =>
  Number.isFinite(value) ? Math.max(1, Math.floor(value)) : 1;

/**
 * Maximizes the image's vertical length first. Extremely wide pages become
 * width-bound, while the media frame always keeps the source aspect ratio.
 */
export function pagePreviewFrame(
  dimension: PagePreviewDimension | undefined,
  viewport: PagePreviewViewport,
): PagePreviewFrame {
  const width = validPagePreviewDimension(dimension) ? dimension.width : FALLBACK_WIDTH;
  const height = validPagePreviewDimension(dimension) ? dimension.height : FALLBACK_HEIGHT;
  const aspect = pagePreviewAspect(dimension);
  const availableWidth = Math.max(1, boundedViewport(viewport.width) - VIEWPORT_MARGIN * 2);
  const availableHeight = Math.max(1, boundedViewport(viewport.height) - VIEWPORT_MARGIN * 2);
  const contentWidth = Math.max(1, availableWidth - DIALOG_BORDER_SIZE);
  const contentHeight = Math.max(1, availableHeight - DIALOG_BORDER_SIZE);
  const chromeHeight = Math.min(
    DIALOG_CHROME_HEIGHT,
    Math.max(0, contentHeight - 1),
  );
  const maximumMediaHeight = Math.max(1, contentHeight - chromeHeight);
  const mediaHeight = Math.min(maximumMediaHeight, contentWidth / aspect);
  const mediaWidth = mediaHeight * aspect;
  const minimumDialogWidth = Math.min(MIN_DIALOG_WIDTH, availableWidth);

  return {
    dialogWidth: Math.round(Math.min(availableWidth, Math.max(minimumDialogWidth, mediaWidth + DIALOG_BORDER_SIZE))),
    dialogHeight: Math.round(Math.min(availableHeight, mediaHeight + chromeHeight + DIALOG_BORDER_SIZE)),
    mediaWidth: Math.round(mediaWidth),
    mediaHeight: Math.round(mediaHeight),
    aspectRatio: `${width} / ${height}`,
    orientation: pagePreviewOrientation({ width, height }) ?? "portrait",
  };
}
