import { tagTooltip } from "../../data/tagTranslations";
import aliases from "./metadataGlossary.ko.json";
import type { PixivTagTranslation } from "./model";
import { kanaReading, safeMetadataText } from "./japaneseReading";

export type MetadataKind = "tag" | "artist" | "group" | "title";
export const METADATA_LABELS = { ko: "한국어 뜻", en: "영어 해석", reading: "영어 발음" } as const;
export type MetadataLine = { label: typeof METADATA_LABELS[keyof typeof METADATA_LABELS]; text: string; language: "ko" | "en"; note?: "추정" | "일부 추정" };

/** Already-readable originals stay in the header, not in a fabricated meaning row. */
export function isReadableMetadataOriginal(text: string): boolean {
  return safeMetadataText(text) && /[\p{Script=Hangul}\p{Script=Latin}]/u.test(text)
    && !/[^\p{Script=Hangul}\p{Script=Latin}\p{Number}\p{Punctuation}\p{Symbol}\p{White_Space}]/u.test(text);
}

export function officialHint(text: string, hints: unknown): PixivTagTranslation | undefined {
  if (!Array.isArray(hints) || hints.length > 70) return;
  const result: PixivTagTranslation = { tag: text };
  for (const value of hints) {
    if (!value || typeof value !== "object" || value.tag !== text) continue;
    for (const field of ["ko", "en", "romaji"] as const) {
      if (!result[field] && safeMetadataText(value[field], 120) && value[field].trim() !== text) result[field] = value[field].trim();
    }
  }
  return result.ko || result.en || result.romaji ? result : undefined;
}

/** Exact-only lookup. A proper name is never translated word-by-word. */
function koreanTag(text: string): string | undefined {
  const value = Object.hasOwn(aliases, text) ? (aliases as Record<string, string>)[text] : undefined;
  if (value) return value;
  // Use the shared dictionary only for an exact English key, not its unknown
  // fallback (which otherwise labels Japanese text as English).
  const shared = tagTooltip(text);
  return shared.language === "ko" ? shared.text : undefined;
}

export function metadataLines(text: string, kind: MetadataKind, hints?: unknown): MetadataLine[] {
  if (!safeMetadataText(text)) return [];
  const hint = kind === "tag" ? officialHint(text, hints) : undefined;
  const korean = hint?.ko ?? (kind === "tag" ? koreanTag(text) ?? (hint?.en ? koreanTag(hint.en) : undefined) : undefined);
  const result: MetadataLine[] = [];
  if (korean) result.push({ label: METADATA_LABELS.ko, text: korean, language: "ko" });
  if (hint?.en) result.push({ label: METADATA_LABELS.en, text: hint.en, language: "en" });
  const reading = hint?.romaji ?? kanaReading(text);
  // Meaning and pronunciation are separate roles, even if their text matches.
  if (reading) result.push({ label: METADATA_LABELS.reading, text: reading, language: "en" });
  return result;
}

