import { useEffect, useMemo, useRef, useState, type FormEvent, type KeyboardEvent } from "react";
import type { Language, SearchUi, ViewId } from "../core/types";
import { languageOrder, languagePresentation } from "../data/languages";
import type { TagNamespace } from "../api/contracts";
import type { SearchSuggestion } from "../search/searchSuggestions";
import { activeSearchToken, replaceActiveSearchToken, searchTokenKind } from "../search/searchTokens";
import { FluentIcon } from "./FluentIcon";

export type { SearchSuggestion } from "../search/searchSuggestions";

const languageOptions = languageOrder.map((value) => ({ value, ...languagePresentation[value] }));
const suggestionNamespaces = new Set<TagNamespace>(["artist", "group", "tag", "female", "male"]);

const isSuggestionNamespace = (value: string | null): value is TagNamespace =>
  value !== null && suggestionNamespaces.has(value as TagNamespace);

const placeholders: Record<ViewId, string> = {
  explore: "앨범, 작가, 그룹, 태그 검색",
  "auto-find": "현재 후보에서 검색",
  downloads: "다운로드 목록에서 검색",
};

type ViewHeaderProps = {
  view: ViewId;
  search: SearchUi;
  searchPending?: boolean;
  suggestions: SearchSuggestion[];
  activityCount: number;
  activityOpen: boolean;
  onDraft: (value: string) => void;
  onSuggestions: (open: boolean, active?: number | null) => void;
  onCommit: (value?: string) => void;
  onSelectSuggestion: (suggestion: SearchSuggestion, value: string) => void;
  onCompleteSuggestion: (value: string) => void;
  onLanguages: (languages: Language[]) => void;
  tagCatalogRevision?: number;
  onTagSuggestionQuery: (query: string, namespace?: TagNamespace) => void;
  onRandomOpen: () => void;
  randomOpenPending: boolean;
  randomOpenAvailable: boolean;
  onActivity: () => void;
  privacyMode: boolean;
  privacyModePending?: boolean;
  onPrivacyModeToggle: () => void;
  onSettings: () => void;
};

export function ViewHeader({
  view,
  search,
  searchPending = false,
  suggestions,
  activityCount,
  activityOpen,
  onDraft,
  onSuggestions,
  onCommit,
  onSelectSuggestion,
  onCompleteSuggestion,
  onLanguages,
  tagCatalogRevision,
  onTagSuggestionQuery,
  onRandomOpen,
  randomOpenPending,
  randomOpenAvailable,
  onActivity,
  privacyMode,
  privacyModePending = false,
  onPrivacyModeToggle,
  onSettings,
}: ViewHeaderProps) {
  const host = useRef<HTMLElement>(null);
  const languageButton = useRef<HTMLButtonElement>(null);
  const input = useRef<HTMLInputElement>(null);
  const composing = useRef(false);
  const [languageOpen, setLanguageOpen] = useState(false);
  const [selection, setSelection] = useState({ start: 0, end: 0 });
  const visibleSuggestions = suggestions;
  const randomOpenDescription = view === "explore"
    ? "Hitomi 전체 범위에서 랜덤 갤러리 열기"
    : view === "auto-find"
      ? "현재 로드된 Auto Find 후보에서 랜덤 열기"
      : "다운로드 완료 앨범에서 랜덤 열기";

  useEffect(() => {
    if (search.activeSuggestion !== null && search.activeSuggestion >= visibleSuggestions.length) {
      onSuggestions(search.suggestionsOpen, null);
    }
  }, [onSuggestions, search.activeSuggestion, search.suggestionsOpen, visibleSuggestions.length]);

  useEffect(() => {
    const closeTransient = (event: PointerEvent) => {
      if (!host.current?.contains(event.target as Node)) {
        setLanguageOpen(false);
        onSuggestions(false);
      }
    };
    const closeOnEscape = (event: globalThis.KeyboardEvent) => {
      if (event.key !== "Escape" || (!languageOpen && !search.suggestionsOpen)) return;
      event.preventDefault();
      event.stopImmediatePropagation();
      const restoreLanguageFocus = languageOpen;
      setLanguageOpen(false);
      onSuggestions(false);
      if (restoreLanguageFocus) window.requestAnimationFrame(() => languageButton.current?.focus());
    };
    document.addEventListener("pointerdown", closeTransient);
    document.addEventListener("keydown", closeOnEscape);
    return () => {
      document.removeEventListener("pointerdown", closeTransient);
      document.removeEventListener("keydown", closeOnEscape);
    };
  }, [languageOpen, onSuggestions, search.suggestionsOpen]);

  useEffect(() => {
    if (view !== "explore" || composing.current || !search.suggestionsOpen) return;
    const raw = activeSearchToken(search.draft, selection.start, selection.end).value.replace(/^-/, "");
    const kind = searchTokenKind(raw);
    if (kind && !isSuggestionNamespace(kind)) { onTagSuggestionQuery("", undefined); return; }
    const value = (kind ? raw.slice(raw.indexOf(":") + 1) : raw).trim();
    if (value.replace(/[\s_]/g, "").length < 2) { onTagSuggestionQuery("", undefined); return; }
    const namespace = isSuggestionNamespace(kind) ? kind : undefined;
    const timer = window.setTimeout(() => onTagSuggestionQuery(value, namespace), 100);
    return () => window.clearTimeout(timer);
  }, [onTagSuggestionQuery, search.draft, search.suggestionsOpen, selection.end, selection.start, tagCatalogRevision, view]);

  const complete = (item: SearchSuggestion, submitNow: boolean) => {
    const caretStart = input.current?.selectionStart ?? selection.start;
    const caretEnd = input.current?.selectionEnd ?? selection.end;
    const nextValue = item.request
      ? item.token
      : replaceActiveSearchToken(search.draft, caretStart, item.token, caretEnd);
    if (item.request || submitNow) onSelectSuggestion(item, nextValue);
    else {
      onCompleteSuggestion(nextValue);
      window.requestAnimationFrame(() => {
        input.current?.focus();
        const nextCaret = nextValue.length;
        input.current?.setSelectionRange(nextCaret, nextCaret);
      });
    }
  };

  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (composing.current) return;
    if (search.activeSuggestion !== null) {
      const item = visibleSuggestions[search.activeSuggestion];
      if (item) {
        complete(item, true);
        return;
      }
    }
    onCommit();
  };

  const keyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.nativeEvent.isComposing || composing.current) return;
    if (event.key === "ArrowDown" && visibleSuggestions.length) {
      event.preventDefault();
      const next = search.activeSuggestion === null ? 0 : (search.activeSuggestion + 1) % visibleSuggestions.length;
      onSuggestions(true, next);
    } else if (event.key === "ArrowUp" && visibleSuggestions.length) {
      event.preventDefault();
      const next = search.activeSuggestion === null
        ? visibleSuggestions.length - 1
        : (search.activeSuggestion - 1 + visibleSuggestions.length) % visibleSuggestions.length;
      onSuggestions(true, next);
    } else if (event.key === "Escape") {
      onSuggestions(false);
    } else if (event.key === "Tab" && search.activeSuggestion !== null) {
      const item = visibleSuggestions[search.activeSuggestion];
      if (!item) return;
      event.preventDefault();
      complete(item, false);
    }
  };

  const toggleLanguage = (language: Language) => {
    const languages = search.languages.includes(language)
      ? search.languages.filter((item) => item !== language)
      : [...search.languages, language];
    onLanguages(languages);
  };

  return (
    <header className="view-header" ref={host}>
      <form id="gallery-search-form" className="search-box" autoComplete="off" onSubmit={submit}>
        <FluentIcon glyph="\uE721" />
        <input
          ref={input}
          type="search"
          role="combobox"
          aria-autocomplete="list"
          value={search.draft}
          placeholder={placeholders[view]}
          aria-label="검색"
          aria-controls="search-suggestions"
          aria-expanded={search.suggestionsOpen && visibleSuggestions.length > 0}
          aria-activedescendant={
            search.activeSuggestion === null ? undefined : `search-suggestion-${search.activeSuggestion}`
          }
          disabled={searchPending}
          onFocus={(event) => {
            setSelection({ start: event.currentTarget.selectionStart ?? 0, end: event.currentTarget.selectionEnd ?? 0 });
            onSuggestions(true);
          }}
          onClick={(event) => setSelection({ start: event.currentTarget.selectionStart ?? 0, end: event.currentTarget.selectionEnd ?? 0 })}
          onChange={(event) => {
            onDraft(event.target.value);
            onSuggestions(true);
            setSelection({ start: event.target.selectionStart ?? event.target.value.length, end: event.target.selectionEnd ?? event.target.value.length });
          }}
          onSelect={(event) => setSelection({ start: event.currentTarget.selectionStart ?? 0, end: event.currentTarget.selectionEnd ?? 0 })}
          onKeyUp={(event) => setSelection({ start: event.currentTarget.selectionStart ?? 0, end: event.currentTarget.selectionEnd ?? 0 })}
          onCompositionStart={() => { composing.current = true; }}
          onCompositionEnd={(event) => { composing.current = false; setSelection({ start: event.currentTarget.selectionStart ?? 0, end: event.currentTarget.selectionEnd ?? 0 }); }}
          onKeyDown={keyDown}
        />
        {search.suggestionsOpen && visibleSuggestions.length ? (
          <div className="suggestions" id="search-suggestions" role="listbox" aria-label="검색 제안">
            {visibleSuggestions.map((item, index) => (
              <button
                key={`${item.type}-${item.token}`}
                id={`search-suggestion-${index}`}
                type="button"
                role="option"
                tabIndex={-1}
                aria-selected={search.activeSuggestion === index}
                className={`suggestion${item.favorite ? " is-favorite" : ""}${
                  search.activeSuggestion === index ? " is-active" : ""
                }`}
                onMouseDown={(event) => event.preventDefault()}
                onClick={() => {
                  complete(item, true);
                }}
              >
                <span className="suggestion-type">{item.type}</span>
                <strong>{item.favorite ? `★ ${item.label}` : item.label}</strong>
                <small>{item.extra}</small>
              </button>
            ))}
          </div>
        ) : null}
      </form>
      <button type="submit" form="gallery-search-form" className="icon-button primary-soft" title="검색" aria-label="검색" disabled={searchPending}>
        <FluentIcon glyph="\uE721" />
      </button>
      <div className="menu-anchor">
        <button
          type="button"
          ref={languageButton}
          className={`icon-button${search.languages.length ? " is-active" : ""}`}
          title="언어 필터"
          aria-label="언어 필터"
          aria-expanded={languageOpen}
          onClick={() => setLanguageOpen((open) => !open)}
        >
          <FluentIcon glyph="\uE774" />
        </button>
        {languageOpen ? (
          <div className="popover language-popover">
            <strong>언어</strong>
            {languageOptions.map((option) => (
              <label key={option.value}>
                <input
                  type="checkbox"
                  checked={search.languages.includes(option.value)}
                  onChange={() => toggleLanguage(option.value)}
                />
                {option.icon ? (
                  <img className="language-option-icon" src={option.icon} alt="" />
                ) : option.fallback ? (
                  <span className="language-option-fallback" aria-hidden="true">{option.fallback}</span>
                ) : null}
                {option.label}
              </label>
            ))}
          </div>
        ) : null}
      </div>
      <button
        type="button"
        className={`icon-button random-open-button${randomOpenPending ? " is-pending" : ""}`}
        title={randomOpenPending ? "랜덤 갤러리를 찾는 중" : randomOpenDescription}
        aria-label={randomOpenPending ? "랜덤 열기 중" : "랜덤 열기"}
        aria-busy={randomOpenPending || undefined}
        disabled={randomOpenPending || !randomOpenAvailable}
        onClick={onRandomOpen}
      >
        {randomOpenPending ? <span className="spinner random-open-spinner" aria-hidden="true" /> : <FluentIcon glyph="\uE8B1" />}
        <span className="random-open-label">{randomOpenPending ? "찾는 중" : "랜덤 열기"}</span>
      </button>
      <button
        type="button"
        className="icon-button activity-button"
        title="활동 기록"
        aria-label="활동 기록"
        aria-controls="activity-panel"
        aria-expanded={activityOpen}
        onClick={onActivity}
      >
        <FluentIcon glyph="\uE9D9" />
        {activityCount > 0 ? <span className="activity-count">{activityCount}</span> : null}
      </button>
      <button
        type="button"
        className={`icon-button${privacyMode ? " is-active" : ""}`}
        title={privacyMode ? "미리보기 표시" : "미리보기 가리기"}
        aria-label="프라이버시 모드"
        aria-pressed={privacyMode}
        aria-busy={privacyModePending || undefined}
        disabled={privacyModePending}
        onClick={onPrivacyModeToggle}
      >
        <FluentIcon glyph="\uE890" />
      </button>
      <button type="button" className="icon-button" title="설정" aria-label="설정" onClick={onSettings}>
        <FluentIcon glyph="\uE713" />
      </button>
    </header>
  );
}
