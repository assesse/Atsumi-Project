import type { PixivHomeSuggestions } from "./model";
import { PixivMetadataText } from "./PixivMetadataText";
import "./PixivHomeHeader.css";

function verifiedTranslations(suggestions: PixivHomeSuggestions | undefined, favorites: string[] | undefined) {
  const result = new Map<string, { translation: string; language: "ko" | "en" }>();
  const rows: unknown = suggestions?.favoriteTagTranslations;
  if (!favorites || !Array.isArray(rows) || rows.length > 10) return result;
  for (const row of rows) {
    if (!row || typeof row !== "object" || typeof row.tag !== "string" || !favorites.includes(row.tag)
      || typeof row.translation !== "string" || (row.language !== "ko" && row.language !== "en")) continue;
    const translation = row.translation.trim();
    if (!translation || translation === row.tag || Array.from(translation).length > 120
      || /[\u0000-\u001f\u007f-\u009f]/u.test(row.translation) || result.has(row.tag)) continue;
    result.set(row.tag, { translation, language: row.language });
  }
  return result;
}

export function PixivHomeHeader({ suggestions, authenticated, disabled, retained, onSearch, onRefresh }: {
  suggestions?: PixivHomeSuggestions; authenticated: boolean; disabled: boolean; retained: boolean;
  onSearch: (tag: string) => void; onRefresh: () => void;
}) {
  const refreshLabel = retained ? "추천 작품 새로고침" : "추천 작품 보기";
  return <>
    <PixivTagGroups suggestions={suggestions} authenticated={authenticated} onSearch={onSearch} />
    <div className="pixiv-home-heading"><h2>추천 작품</h2><button type="button" aria-label={refreshLabel} title={refreshLabel} disabled={disabled} onClick={onRefresh}>
      <svg viewBox="0 0 24 24" width="20" height="20" aria-hidden="true" fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round"><path d="M20 8a8 8 0 1 0 0 8M20 3v5h-5" /></svg>
    </button></div>
  </>;
}

export function PixivTagGroups({ suggestions, authenticated, onSearch, searchPanel = false }: {
  suggestions?: PixivHomeSuggestions; authenticated: boolean; onSearch: (tag: string) => void; searchPanel?: boolean;
}) {
  const favoriteTags = authenticated && Array.isArray(suggestions?.favoriteTags) && suggestions.favoriteTags.every(tag => typeof tag === "string") ? suggestions.favoriteTags.slice(0, 10) : undefined;
  const favoriteUnavailable = authenticated && suggestions !== undefined && favoriteTags === undefined;
  const translations = verifiedTranslations(suggestions, favoriteTags);
  const groups = suggestions ? [
    { label: "즐겨찾기 태그", tags: favoriteTags ?? [] },
    { label: "추천 검색어", tags: suggestions.recommendedTags.slice(0, 8) },
    { label: searchPanel ? "인기 일러스트 태그" : "인기 태그", tags: suggestions.popularTags.slice(0, 12) },
    ...(searchPanel ? [{ label: "인기 소설 태그", tags: (suggestions.popularNovelTags ?? []).slice(0, 12) }] : []),
  ].filter(group => group.tags.length > 0) : [];
  return <>
    {favoriteUnavailable && <p className="pixiv-favorite-tags-error" role="status">즐겨찾기 태그를 불러오지 못했습니다. 추천 작품을 새로고침해 주세요.</p>}
    {groups.length > 0 && <div className="pixiv-discovery-tags" aria-label="Pixiv 추천 검색어">
      {groups.map(group => <div key={group.label}><span>{group.label}</span><div>
        {group.tags.map(tag => {
          const translated = group.label === "즐겨찾기 태그" ? translations.get(tag) : undefined;
          const hints = [...(Array.isArray(suggestions?.tagTranslations) && suggestions.tagTranslations.length <= 70 ? suggestions.tagTranslations.filter(row => row?.tag === tag) : []), ...(translated ? [{ tag, [translated.language]: translated.translation }] : [])];
          return <PixivMetadataText as="button" text={tag} kind="tag" hints={hints} key={tag} className={translated ? "pixiv-favorite-tag-translated" : undefined} onClick={() => onSearch(tag)}>
            <span className="pixiv-tag-original">{tag}</span>
            {translated && <span className="pixiv-tag-translation" lang={translated.language}>{translated.translation}</span>}
          </PixivMetadataText>;
        })}
      </div></div>)}
    </div>}
  </>;
}
