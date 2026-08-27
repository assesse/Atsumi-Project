import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi } from "vitest";
import { ViewHeader } from "./ViewHeader";

describe("ViewHeader language filter", () => {
  it("uses an active state without a numeric badge and preserves the activity count", async () => {
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    try {
      await act(async () => root.render(
        <ViewHeader
          view="explore"
          search={{ draft: "", committed: "", languages: ["korean", "english"], suggestionsOpen: false, activeSuggestion: null }}
          suggestions={[]}
          activityCount={3}
          activityOpen={false}
          onDraft={vi.fn()}
          onSuggestions={vi.fn()}
          onCommit={vi.fn()}
          onSelectSuggestion={vi.fn()}
          onCompleteSuggestion={vi.fn()}
          onLanguages={vi.fn()}
          onTagCatalogRefresh={vi.fn()}
          tagCatalogStatus={{ revision: 0, entryCount: 0, neutralCount: 0, femaleCount: 0, maleCount: 0, artistCount: 0, groupCount: 0 }}
          tagCatalogRefreshing={false}
          tagCatalogRevision={0}
          onTagSuggestionQuery={vi.fn()}
          onActivity={vi.fn()}
          privacyMode={false}
          onPrivacyModeToggle={vi.fn()}
          onSettings={vi.fn()}
        />,
      ));
      const languageButton = container.querySelector('button[aria-label="언어 필터"]');
      expect(languageButton?.querySelector(".icon-dot")).toBeNull();
      expect(container.querySelector(".activity-count")).toHaveTextContent("3");
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });

  it("shows an in-button busy indicator while the global tag catalog refresh runs", async () => {
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    try {
      await act(async () => root.render(
        <ViewHeader
          view="explore"
          search={{ draft: "", committed: "", languages: [], suggestionsOpen: false, activeSuggestion: null }}
          suggestions={[]}
          activityCount={0}
          activityOpen={false}
          onDraft={vi.fn()}
          onSuggestions={vi.fn()}
          onCommit={vi.fn()}
          onSelectSuggestion={vi.fn()}
          onCompleteSuggestion={vi.fn()}
          onLanguages={vi.fn()}
          onTagCatalogRefresh={vi.fn()}
          tagCatalogStatus={{ revision: 1, entryCount: 10_000, neutralCount: 4_000, femaleCount: 3_000, maleCount: 1_000, artistCount: 1_500, groupCount: 500 }}
          tagCatalogRefreshing
          tagCatalogRevision={1}
          onTagSuggestionQuery={vi.fn()}
          onActivity={vi.fn()}
          privacyMode={false}
          onPrivacyModeToggle={vi.fn()}
          onSettings={vi.fn()}
        />,
      ));
      const button = container.querySelector<HTMLButtonElement>('button[aria-label="검색 자동완성 최신화 중"]');
      expect(button).toBeDisabled();
      expect(button).toHaveAttribute("aria-busy", "true");
      expect(button).toHaveClass("is-refreshing");
      expect(button?.querySelector(".catalog-refresh-spinner")).not.toBeNull();
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });

  it("completes only the active token on Tab and submits it on Enter", async () => {
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const onSelectSuggestion = vi.fn();
    const onCompleteSuggestion = vi.fn();
    const suggestion = { type: "TAG" as const, token: "tag:full_color", label: "full color", extra: "태그" };
    try {
      await act(async () => root.render(
        <ViewHeader view="explore" search={{ draft: "artist:mizuno tag:full", committed: "", languages: [], suggestionsOpen: true, activeSuggestion: 0 }} suggestions={[suggestion]} activityCount={0} activityOpen={false} onDraft={vi.fn()} onSuggestions={vi.fn()} onCommit={vi.fn()} onSelectSuggestion={onSelectSuggestion} onCompleteSuggestion={onCompleteSuggestion} onLanguages={vi.fn()} onTagCatalogRefresh={vi.fn()} tagCatalogStatus={{ revision: 1, entryCount: 11, neutralCount: 1, femaleCount: 5, maleCount: 1, artistCount: 2, groupCount: 2 }} tagCatalogRefreshing={false} tagCatalogRevision={1} onTagSuggestionQuery={vi.fn()} onActivity={vi.fn()} privacyMode={false} onPrivacyModeToggle={vi.fn()} onSettings={vi.fn()} />,
      ));
      const input = container.querySelector<HTMLInputElement>('input[aria-label="검색"]');
      if (!input) throw new Error("search input missing");
      input.setSelectionRange(23, 23);
      await act(async () => input.dispatchEvent(new KeyboardEvent("keyup", { key: "End", bubbles: true })));
      await act(async () => container.querySelector("form")?.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
      expect(onSelectSuggestion).toHaveBeenCalledWith(suggestion, "artist:mizuno tag:full_color");
      onSelectSuggestion.mockClear();
      await act(async () => input.dispatchEvent(new KeyboardEvent("keydown", { key: "Tab", bubbles: true })));
      expect(onCompleteSuggestion).toHaveBeenCalledWith("artist:mizuno tag:full_color");
      expect(onSelectSuggestion).not.toHaveBeenCalled();
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });

  it("queries artist and group catalogs for the active namespaced token", async () => {
    vi.useFakeTimers();
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const onTagSuggestionQuery = vi.fn();
    const renderWithDraft = async (draft: string) => {
      await act(async () => root.render(
        <ViewHeader
          view="explore"
          search={{ draft, committed: "", languages: [], suggestionsOpen: true, activeSuggestion: null }}
          suggestions={[]}
          activityCount={0}
          activityOpen={false}
          onDraft={vi.fn()}
          onSuggestions={vi.fn()}
          onCommit={vi.fn()}
          onSelectSuggestion={vi.fn()}
          onCompleteSuggestion={vi.fn()}
          onLanguages={vi.fn()}
          onTagCatalogRefresh={vi.fn()}
          tagCatalogStatus={{ revision: 1, entryCount: 11, neutralCount: 1, femaleCount: 5, maleCount: 1, artistCount: 2, groupCount: 2 }}
          tagCatalogRefreshing={false}
          tagCatalogRevision={1}
          onTagSuggestionQuery={onTagSuggestionQuery}
          onActivity={vi.fn()}
          privacyMode={false}
          onPrivacyModeToggle={vi.fn()}
          onSettings={vi.fn()}
        />,
      ));
      const input = container.querySelector<HTMLInputElement>('input[aria-label="검색"]');
      if (!input) throw new Error("search input missing");
      input.setSelectionRange(draft.length, draft.length);
      await act(async () => input.dispatchEvent(new Event("select", { bubbles: true })));
      await act(async () => vi.advanceTimersByTime(101));
    };

    try {
      await renderWithDraft("artist:miz");
      expect(onTagSuggestionQuery).toHaveBeenLastCalledWith("miz", "artist");

      await renderWithDraft("group:circle_na");
      expect(onTagSuggestionQuery).toHaveBeenLastCalledWith("circle_na", "group");

      await renderWithDraft("series:rain");
      expect(onTagSuggestionQuery).toHaveBeenLastCalledWith("", undefined);
    } finally {
      await act(async () => root.unmount());
      container.remove();
      vi.useRealTimers();
    }
  });

  it("exposes the persistent privacy toggle as a pressed, busy-aware button", async () => {
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const onPrivacyModeToggle = vi.fn();
    try {
      await act(async () => root.render(
        <ViewHeader
          view="downloads"
          search={{ draft: "", committed: "", languages: [], suggestionsOpen: false, activeSuggestion: null }}
          suggestions={[]}
          activityCount={0}
          activityOpen={false}
          onDraft={vi.fn()}
          onSuggestions={vi.fn()}
          onCommit={vi.fn()}
          onSelectSuggestion={vi.fn()}
          onCompleteSuggestion={vi.fn()}
          onLanguages={vi.fn()}
          onTagCatalogRefresh={vi.fn()}
          tagCatalogStatus={{ revision: 1, entryCount: 3, neutralCount: 1, femaleCount: 0, maleCount: 0, artistCount: 1, groupCount: 1 }}
          tagCatalogRefreshing={false}
          tagCatalogRevision={1}
          onTagSuggestionQuery={vi.fn()}
          onActivity={vi.fn()}
          privacyMode
          onPrivacyModeToggle={onPrivacyModeToggle}
          onSettings={vi.fn()}
        />,
      ));
      const toggle = container.querySelector<HTMLButtonElement>('[aria-label="개인정보 보호 모드"]');
      expect(toggle).toHaveAttribute("aria-pressed", "true");
      expect(toggle).toHaveClass("is-active");
      await act(async () => toggle?.click());
      expect(onPrivacyModeToggle).toHaveBeenCalledOnce();

      await act(async () => root.render(
        <ViewHeader
          view="downloads"
          search={{ draft: "", committed: "", languages: [], suggestionsOpen: false, activeSuggestion: null }}
          suggestions={[]}
          activityCount={0}
          activityOpen={false}
          onDraft={vi.fn()}
          onSuggestions={vi.fn()}
          onCommit={vi.fn()}
          onSelectSuggestion={vi.fn()}
          onCompleteSuggestion={vi.fn()}
          onLanguages={vi.fn()}
          onTagCatalogRefresh={vi.fn()}
          tagCatalogStatus={{ revision: 1, entryCount: 3, neutralCount: 1, femaleCount: 0, maleCount: 0, artistCount: 1, groupCount: 1 }}
          tagCatalogRefreshing={false}
          tagCatalogRevision={1}
          onTagSuggestionQuery={vi.fn()}
          onActivity={vi.fn()}
          privacyMode
          privacyModePending
          onPrivacyModeToggle={onPrivacyModeToggle}
          onSettings={vi.fn()}
        />,
      ));
      const pendingToggle = container.querySelector<HTMLButtonElement>('[aria-label="개인정보 보호 모드"]');
      expect(pendingToggle).toBeDisabled();
      expect(pendingToggle).toHaveAttribute("aria-busy", "true");
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });
});
