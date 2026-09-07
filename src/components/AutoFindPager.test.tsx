import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi } from "vitest";
import { AutoFindPager } from "./AutoFindPager";

describe("AutoFindPager", () => {
  it("moves only within the available Auto Find pages", async () => {
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const onPageChange = vi.fn();
    try {
      await act(async () => root.render(
        <AutoFindPager page={2} totalPages={4} onPageChange={onPageChange} />,
      ));
      expect(container.querySelector("nav")).toHaveAccessibleName("Auto Find 페이지");
      expect(container).toHaveTextContent("2 / 4");
      expect(container).not.toHaveTextContent("전체 73개");
      const buttons = [...container.querySelectorAll<HTMLButtonElement>("button")];
      await act(async () => buttons.find((button) => button.textContent === "이전")?.click());
      await act(async () => buttons.find((button) => button.textContent === "다음")?.click());
      expect(onPageChange.mock.calls).toEqual([[1], [3]]);
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });

  it("accepts a direct page number and clamps it on Enter or blur", async () => {
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const onPageChange = vi.fn();
    try {
      await act(async () => root.render(
        <AutoFindPager
          page={2}
          totalPages={7}
          onPageChange={onPageChange}
          ariaLabel="다운로드 목록 페이지"
        />,
      ));
      const input = container.querySelector<HTMLInputElement>(".pager-page-input");
      if (!input) throw new Error("direct page input was not rendered");
      expect(input).toHaveAccessibleName("다운로드 목록 페이지 번호 직접 입력");
      expect(input).toHaveValue(2);

      await act(async () => {
        Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set?.call(input, "5");
        input.dispatchEvent(new Event("input", { bubbles: true }));
      });
      await act(async () => input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })));
      expect(onPageChange).toHaveBeenLastCalledWith(5);

      await act(async () => root.render(
        <AutoFindPager
          page={5}
          totalPages={7}
          onPageChange={onPageChange}
          ariaLabel="다운로드 목록 페이지"
        />,
      ));
      const updatedInput = container.querySelector<HTMLInputElement>(".pager-page-input");
      if (!updatedInput) throw new Error("updated direct page input was not rendered");
      await act(async () => {
        Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set?.call(updatedInput, "99");
        updatedInput.dispatchEvent(new Event("input", { bubbles: true }));
      });
      await act(async () => updatedInput.dispatchEvent(new FocusEvent("focusout", { bubbles: true })));
      expect(onPageChange).toHaveBeenLastCalledWith(7);
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });

  it("locks every navigation control while a page request is busy", async () => {
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    try {
      await act(async () => root.render(
        <AutoFindPager page={2} totalPages={4} onPageChange={vi.fn()} busy />,
      ));
      expect(container.querySelectorAll("button:not(:disabled)")).toHaveLength(0);
      expect(container.querySelector("input")).toBeDisabled();
      expect(container.querySelector("[role='status']")).toHaveTextContent("불러오는 중");
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });

  it("cancels a draft with Escape without committing the stale value during blur", async () => {
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const onPageChange = vi.fn();
    try {
      await act(async () => root.render(
        <AutoFindPager page={2} totalPages={8} onPageChange={onPageChange} />,
      ));
      const input = container.querySelector<HTMLInputElement>(".pager-page-input");
      if (!input) throw new Error("direct page input was not rendered");
      input.focus();
      await act(async () => {
        Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set?.call(input, "7");
        input.dispatchEvent(new Event("input", { bubbles: true }));
      });
      await act(async () => {
        input.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
        await Promise.resolve();
      });

      expect(onPageChange).not.toHaveBeenCalled();
      expect(input).toHaveValue(2);
      expect(input).not.toHaveFocus();
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });

  it("hides when every candidate fits on one page", async () => {
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    try {
      await act(async () => root.render(
        <AutoFindPager page={1} totalPages={1} onPageChange={vi.fn()} />,
      ));
      expect(container).toBeEmptyDOMElement();
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });
});
