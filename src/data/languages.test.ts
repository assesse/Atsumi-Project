import { describe, expect, it } from "vitest";
import { languagePresentation } from "./languages";

describe("Classic language assets", () => {
  it("uses PNG assets captured from the Classic runtime for its three countries", () => {
    const icons = [
      languagePresentation.korean.icon,
      languagePresentation.japanese.icon,
      languagePresentation.english.icon,
    ];

    const isPngAssetReference = (icon: string | null) =>
      Boolean(
        icon &&
          (icon.startsWith("data:image/png") ||
            /\/bundled\/(?:kr|jp|us)\.png(?:\?|$)/.test(icon)),
      );

    expect(icons.every(isPngAssetReference)).toBe(true);
    expect(new Set(icons).size).toBe(3);
  });

  it("provides a local CN flag and fallback for Chinese", () => {
    expect(languagePresentation.chinese.icon).toMatch(/^data:image\/svg\+xml/);
    expect(languagePresentation.chinese.fallback).toBe("CN");
  });
});
