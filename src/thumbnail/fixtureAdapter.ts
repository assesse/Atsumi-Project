import mockGallerySheet from "../assets/mock-gallery-sheet.png";
import type { ThumbnailCoordinatorAdapter, ThumbnailSpriteAsset } from "./client";

const SHEET_WIDTH = 1536;
const SHEET_HEIGHT = 1024;
const SHEET_COLUMNS = 3;
const SHEET_ROWS = 2;

/** Browser-only presentation fixture. It performs no network or cache work. */
export const browserFixtureThumbnailAdapter: ThumbnailCoordinatorAdapter = {
  resolve({ key }): ThumbnailSpriteAsset {
    return {
      kind: "sprite",
      url: mockGallerySheet,
      sheetWidth: SHEET_WIDTH,
      sheetHeight: SHEET_HEIGHT,
      columns: SHEET_COLUMNS,
      rows: SHEET_ROWS,
      cell: key.fallback?.index ?? 0,
    };
  },
};
