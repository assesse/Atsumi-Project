import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi } from "vitest";
import { ViewHeader } from "./ViewHeader";

describe("ViewHeader language filter", () => {
  it("uses an active state without a numeric badge and preserves the activity count", async () => {
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const onRandomOpen = vi.fn();
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
          tagCatalogRevision={0}
          onTagSuggestionQuery={vi.fn()}
          onRandomOpen={onRandomOpen}
          randomOpenPending={false}
          randomOpenAvailable
          onActivity={vi.fn()}
          privacyMode={false}
          onPrivacyModeToggle={vi.fn()}
          onSettings={vi.fn()}
        />,
      ));
      const languageButton = container.querySelector('button[aria-label="언어 필터"]');
      expect(languageButton?.querySelector(".icon-dot")).toBeNull();
      expect(container.querySelector(".activity-count")).toHaveTextContent("3");
      const randomOpen = container.querySelector<HTMLButtonElement>('button[aria-label="랜덤 열기"]');
      expect(randomOpen).toHaveAttribute("title", "Hitomi 전체 범위에서 랜덤 갤러리 열기");
      expect(randomOpen?.querySelector(".random-open-label")).toHaveTextContent("랜덤 열기");
      await act(async () => randomOpen?.click());
      expect(onRandomOpen).toHaveBeenCalledOnce();
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });

  it("opens a random gallery and shows an in-button busy indicator while it loads", async () => {
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
          tagCatalogRevision={1}
          onTagSuggestionQuery={vi.fn()}
          onRandomOpen={vi.fn()}
          randomOpenPending
          randomOpenAvailable
          onActivity={vi.fn()}
          privacyMode={false}
          onPrivacyModeToggle={vi.fn()}
          onSettings={vi.fn()}
        />,
      ));
      const button = container.querySelector<HTMLButtonElement>('button[aria-label="랜덤 열기 중"]');
      expect(button).toBeDisabled();
      expect(button).toHaveAttribute("aria-busy", "true");
      expect(button).toHaveClass("is-pending");
      expect(button?.querySelector(".random-open-spinner")).not.toBeNull();
      expect(button?.querySelector(".random-open-label")).toHaveTextContent("찾는 중");
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
        <ViewHeader view="explore" search={{ draft: "artist:mizuno tag:full", committed: "", languages: [], suggestionsOpen: true, activeSuggestion: 0 }} suggestions={[suggestion]} activityCount={0} activityOpen={false} onDraft={vi.fn()} onSuggestions={vi.fn()} onCommit={vi.fn()} onSelectSuggestion={onSelectSuggestion} onCompleteSuggestion={onCompleteSuggestion} onLanguages={vi.fn()} tagCatalogRevision={1} onTagSuggestionQuery={vi.fn()} onRandomOpen={vi.fn()} randomOpenPending={false} randomOpenAvailable onActivity={vi.fn()} privacyMode={false} onPrivacyModeToggle={vi.fn()} onSettings={vi.fn()} />,
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
          tagCatalogRevision={1}
          onTagSuggestionQuery={onTagSuggestionQuery}
          onRandomOpen={vi.fn()}
          randomOpenPending={false}
          randomOpenAvailable
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
          tagCatalogRevision={1}
          onTagSuggestionQuery={vi.fn()}
          onRandomOpen={vi.fn()}
          randomOpenPending={false}
          randomOpenAvailable
          onActivity={vi.fn()}
          privacyMode
          onPrivacyModeToggle={onPrivacyModeToggle}
          onSettings={vi.fn()}
        />,
      ));
      const toggle = container.querySelector<HTMLButtonElement>('[aria-label="프라이버시 모드"]');
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
          tagCatalogRevision={1}
          onTagSuggestionQuery={vi.fn()}
          onRandomOpen={vi.fn()}
          randomOpenPending={false}
          randomOpenAvailable
          onActivity={vi.fn()}
          privacyMode
          privacyModePending
          onPrivacyModeToggle={onPrivacyModeToggle}
          onSettings={vi.fn()}
        />,
      ));
      const pendingToggle = container.querySelector<HTMLButtonElement>('[aria-label="프라이버시 모드"]');
      expect(pendingToggle).toBeDisabled();
      expect(pendingToggle).toHaveAttribute("aria-busy", "true");
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });
});
