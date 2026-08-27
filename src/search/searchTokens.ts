export type SearchTokenKind = "artist" | "group" | "series" | "character" | "tag" | "female" | "male";

const knownKinds = new Set<SearchTokenKind>(["artist", "group", "series", "character", "tag", "female", "male"]);

export function normalizeTokenValue(value: string): string {
  return value.trim().toLocaleLowerCase().replace(/\\_/g, "_").replace(/[\s_]+/g, "_");
}

export function searchTokenKind(value: string): SearchTokenKind | null {
  const separator = value.indexOf(":");
  if (separator < 1) return null;
  const kind = value.slice(0, separator).toLocaleLowerCase() as SearchTokenKind;
  return knownKinds.has(kind) ? kind : null;
}

export function canonicalSearchToken(value: string, defaultKind?: SearchTokenKind): string {
  const negative = value.trimStart().startsWith("-") ? "-" : "";
  const raw = value.trim().replace(/^-/, "");
  const kind = searchTokenKind(raw) ?? defaultKind;
  const tokenValue = normalizeTokenValue(kind ? raw.slice(raw.indexOf(":") + 1) : raw);
  if (!tokenValue) return negative;
  return kind ? `${negative}${kind}:${tokenValue}` : `${negative}${tokenValue}`;
}

export type MetadataSearchToken = Readonly<{
  displayToken: string;
  includeTag?: string;
}>;

/** Converts card metadata into a display token and, for tags, its backend includeTags value. */
export function metadataSearchToken(value: string, searchValue?: string): MetadataSearchToken {
  const candidate = searchValue ?? value;
  const kind = searchTokenKind(candidate);
  if (kind === "female" || kind === "male") {
    const displayToken = canonicalSearchToken(candidate, kind);
    return { displayToken, includeTag: displayToken.replace(/^-/, "").replace(/_+/g, " ").replace(/^(female|male):\s*/, "$1:") };
  }
  if (kind === "tag") {
    const displayToken = canonicalSearchToken(candidate, "tag");
    return { displayToken, includeTag: displayToken.replace(/^-?tag:/, "").replaceAll("_", " ") };
  }
  if (kind) return { displayToken: canonicalSearchToken(candidate, kind) };
  const displayToken = canonicalSearchToken(candidate, "tag");
  return { displayToken, includeTag: displayToken.replace(/^tag:/, "").replaceAll("_", " ") };
}

export type ActiveSearchToken = Readonly<{ start: number; end: number; value: string }>;

export function activeSearchToken(input: string, caretStart: number, caretEnd = caretStart): ActiveSearchToken {
  const startSelection = Math.max(0, Math.min(input.length, caretStart));
  const endSelection = Math.max(startSelection, Math.min(input.length, caretEnd));
  if (startSelection !== endSelection) return { start: startSelection, end: endSelection, value: input.slice(startSelection, endSelection) };
  let start = startSelection;
  let end = endSelection;
  while (start > 0 && !/\s/.test(input[start - 1]!)) start -= 1;
  while (end < input.length && !/\s/.test(input[end]!)) end += 1;
  return { start, end, value: input.slice(start, end) };
}

export function replaceActiveSearchToken(input: string, caretStart: number, replacement: string, caretEnd = caretStart): string {
  const active = activeSearchToken(input, caretStart, caretEnd);
  const preserveNegative = active.value.startsWith("-") && !replacement.startsWith("-");
  return `${input.slice(0, active.start)}${preserveNegative ? "-" : ""}${replacement}${input.slice(active.end)}`;
}
