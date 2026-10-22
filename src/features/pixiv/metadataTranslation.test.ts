import { describe, expect, it } from "vitest";
import { kanaReading, romanizeTokens, safeMetadataText } from "./japaneseReading";
import { isReadableMetadataOriginal, metadataLines, officialHint } from "./metadataTranslation";

describe("Pixiv display-only metadata translation", () => {
  it("prefers exact official Korean, keeps English and official reading distinct", () => {
    const hints = [{ tag: "風景", ko: "경관", en: "landscape", romaji: "fuukei" }];
    expect(metadataLines("風景", "tag", hints)).toEqual([
      { label: "한국어 뜻", text: "경관", language: "ko" },
      { label: "영어 해석", text: "landscape", language: "en" },
      { label: "영어 발음", text: "fuukei", language: "en" },
    ]);
    expect(hints[0]!.tag).toBe("風景");
    expect(metadataLines("風", "tag", hints).some(row => row.text === "경관")).toBe(false);
  });
  it("reuses the Korean glossary through a same-response English alias", () => {
    expect(metadataLines("未知のタグ", "tag", [{ tag: "未知のタグ", en: "cat" }])[0]).toMatchObject({ label: "한국어 뜻", text: "고양이" });
    expect(metadataLines("イラスト", "tag")[0]).toMatchObject({ language: "ko", text: "일러스트" });
  });
  it.each(["artist", "group", "title"] as const)("never semantically translates %s proper names", kind => {
    expect(metadataLines("風景", kind, [{ tag: "風景", ko: "풍경" }])).toEqual([]);
    expect(metadataLines("ミドリ", kind)).toEqual([{ label: "영어 발음", text: "midori", language: "en" }]);
  });
  it("retains distinct meaning and pronunciation roles when values match", () => {
    expect(metadataLines("サクラ", "tag", [{ tag: "サクラ", en: "sakura", romaji: "sakura" }])).toEqual([
      { label: "영어 해석", text: "sakura", language: "en" },
      { label: "영어 발음", text: "sakura", language: "en" },
    ]);
  });
  it("merges only missing fields from duplicate exact hints", () => {
    expect(officialHint("猫", [{ tag: "猫", ko: "bad\nline", en: "cat" }, { tag: "猫", ko: "고양이", en: "replacement" }])).toEqual({ tag: "猫", ko: "고양이", en: "cat" });
  });
  it.each([undefined, null, {}, 5, "bad", Array.from({ length: 71 }, () => ({ tag: "猫", ko: "고양이" }))])("rejects malformed or excessive hints %j", hints => {
    expect(officialHint("猫", hints)).toBeUndefined();
  });
  it.each(["", " ", "猫", "bad\nline", "x\t", "\u202ehi", "x".repeat(121), null, {}, 12])("rejects invalid fields %j", ko => {
    expect(officialHint("猫", [{ tag: "猫", ko }])).toBeUndefined();
  });
  it("keeps Unicode code-point limits and never labels unknown text as English", () => {
    expect(safeMetadataText("𠮷".repeat(120), 120)).toBe(true);
    expect(metadataLines("𠮷𠮷", "artist")).toEqual([]);
    expect(metadataLines("작가", "artist")).toEqual([]);
    expect(metadataLines("Artist", "artist")).toEqual([]);
    expect(isReadableMetadataOriginal("작가 Artist 123")).toBe(true);
    expect(isReadableMetadataOriginal("山田 Artist")).toBe(false);
    expect(isReadableMetadataOriginal("Artist\u202e")).toBe(false);
  });
  it("romanizes Kana and marks unresolved Kanji as partial rather than fabricating it", () => {
    expect(kanaReading("ﾐﾄﾞﾘ")).toBe("midori");
    expect(kanaReading("山田")).toBeUndefined();
    expect(romanizeTokens([{ surface_form: "山田", reading: "ヤマダ" }, { surface_form: "𠮷" }])).toEqual({ text: "yamada 𠮷", partial: true });
    expect(romanizeTokens([{ surface_form: "𠮷" }])).toBeUndefined();
    expect(romanizeTokens(Array.from({ length: 301 }, () => ({ surface_form: "山", reading: "ヤマ" })))).toBeUndefined();
  });
});
