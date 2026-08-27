import rawTranslations from "./tagTranslations.ko.json";

export type TagTooltipLanguage = "ko" | "en";

const normalizeComparable = (value: string): string => value
  .trim()
  .toLocaleLowerCase()
  .replaceAll("_", " ")
  .split(/\s+/)
  .filter(Boolean)
  .join(" ");

/** Normalizes a tag independently from the Hitomi gender namespace. */
export const normalizeTagTranslationKey = (value: string): string => {
  const withoutNamespace = value.trim().replace(/^(?:tag|female|male):/i, "");
  return normalizeComparable(withoutNamespace);
};

const translations = new Map<string, string>(
  Object.entries(rawTranslations as Record<string, string>).map(([key, value]) => [
    normalizeTagTranslationKey(key),
    value,
  ]),
);

export const tagTranslationEntryCount = Object.keys(rawTranslations).length;

export const tagTooltip = (value: string): {
  key: string;
  text: string;
  language: TagTooltipLanguage;
} => {
  const key = normalizeTagTranslationKey(value);
  const candidate = translations.get(key)?.trim() ?? "";
  const translated = candidate.length > 0 && normalizeComparable(candidate) !== normalizeComparable(key);
  return { key, text: translated ? candidate : key, language: translated ? "ko" : "en" };
};
