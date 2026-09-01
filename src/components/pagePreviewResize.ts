export type PagePreviewResizeEdge = "right" | "bottom" | "corner";

export type PagePreviewResizeBox = Readonly<{
  left: number;
  top: number;
  width: number;
  height: number;
}>;

export type PagePreviewResizeViewport = Readonly<{
  width: number;
  height: number;
}>;

const VIEWPORT_MARGIN = 12;
const MIN_SIZE = 320;

const finite = (value: number, fallback: number): number =>
  Number.isFinite(value) ? value : fallback;

const clamp = (value: number, minimum: number, maximum: number): number =>
  Math.min(maximum, Math.max(minimum, value));

export const pagePreviewResizeBounds = (
  viewport: PagePreviewResizeViewport,
) => {
  const viewportWidth = Math.max(1, Math.floor(finite(viewport.width, 1)));
  const viewportHeight = Math.max(1, Math.floor(finite(viewport.height, 1)));
  const maximumWidth = Math.max(1, viewportWidth - VIEWPORT_MARGIN * 2);
  const maximumHeight = Math.max(1, viewportHeight - VIEWPORT_MARGIN * 2);
  return {
    minimumWidth: Math.min(MIN_SIZE, maximumWidth),
    minimumHeight: Math.min(MIN_SIZE, maximumHeight),
    maximumWidth,
    maximumHeight,
  } as const;
};

export function resizePagePreviewBox(
  start: PagePreviewResizeBox,
  edge: PagePreviewResizeEdge,
  deltaX: number,
  deltaY: number,
  viewport: PagePreviewResizeViewport,
): PagePreviewResizeBox {
  const viewportWidth = Math.max(1, Math.floor(finite(viewport.width, 1)));
  const viewportHeight = Math.max(1, Math.floor(finite(viewport.height, 1)));
  const bounds = pagePreviewResizeBounds(viewport);
  const changesWidth = edge === "right" || edge === "corner";
  const changesHeight = edge === "bottom" || edge === "corner";
  const requestedWidth = finite(start.width, bounds.minimumWidth) + (changesWidth ? finite(deltaX, 0) * 2 : 0);
  const requestedHeight = finite(start.height, bounds.minimumHeight) + (changesHeight ? finite(deltaY, 0) * 2 : 0);
  const width = Math.round(clamp(requestedWidth, bounds.minimumWidth, bounds.maximumWidth));
  const height = Math.round(clamp(requestedHeight, bounds.minimumHeight, bounds.maximumHeight));
  return {
    left: Math.round((viewportWidth - width) / 2),
    top: Math.round((viewportHeight - height) / 2),
    width,
    height,
  };
}

export function clampPagePreviewResizeBox(
  box: PagePreviewResizeBox,
  viewport: PagePreviewResizeViewport,
): PagePreviewResizeBox {
  return resizePagePreviewBox(box, "corner", 0, 0, viewport);
}
