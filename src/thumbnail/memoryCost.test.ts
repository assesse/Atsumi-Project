import { describe, expect, it } from "vitest";
import { thumbnailMemoryCost } from "./memoryCost";

describe("thumbnail display memory budget", () => {
  it("accounts for both compressed bytes and decoded pixels", () => {
    expect(thumbnailMemoryCost({ kind: "image", url: "blob:test", width: 1024, height: 1024, byteLength: 100 }))
      .toBe(4 * 1024 * 1024 + 100);
  });
  it("counts an entire sprite sheet, not just its displayed cell", () => {
    expect(thumbnailMemoryCost({ kind: "sprite", url: "blob:sheet", sheetWidth: 2048, sheetHeight: 2048,
      columns: 4, rows: 4, cell: 0 })).toBe(2048 * 2048 * 4.5);
  });
});
