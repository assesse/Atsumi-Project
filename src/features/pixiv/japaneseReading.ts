import { toRomaji } from "wanakana";

export const hasKanji = (value: string) => /\p{Script=Han}/u.test(value);
export const safeMetadataText = (value: unknown, limit = 300): value is string => typeof value === "string" && value.trim().length > 0
  && [...value].length <= limit && !/[\u0000-\u001f\u007f-\u009f\u202a-\u202e\u2066-\u2069]/u.test(value);

export function kanaReading(text: string): string | undefined {
  if (!safeMetadataText(text) || hasKanji(text) || !/[\p{Script=Hiragana}\p{Script=Katakana}]/u.test(text)) return;
  const result = toRomaji(text.normalize("NFKC"));
  return /[\p{Script=Hiragana}\p{Script=Katakana}\p{Script=Han}]/u.test(result) ? undefined : result;
}

export type ReadingToken = { surface_form: string; reading?: string; pronunciation?: string };
export function romanizeTokens(tokens: ReadingToken[]): { text: string; partial: boolean } | undefined {
  if (tokens.length > 300) return;
  let partial = false, changed = false;
  const result = tokens.map(token => {
    const kana = token.pronunciation && token.pronunciation !== "*" ? token.pronunciation : token.reading;
    const source = kana && kana !== "*" ? kana : token.surface_form;
    if (!safeMetadataText(source)) return token.surface_form.trim() ? "" : token.surface_form;
    const reading = toRomaji(source.normalize("NFKC"));
    if (/[\p{Script=Han}\p{Script=Hiragana}\p{Script=Katakana}]/u.test(reading)) partial = true;
    if (reading !== token.surface_form && /[A-Za-z]/.test(reading)) changed = true;
    return reading;
  }).join(" ").replace(/\s+/g, " ").trim();
  return changed && result && safeMetadataText(result, 1600) ? { text: result, partial } : undefined;
}
