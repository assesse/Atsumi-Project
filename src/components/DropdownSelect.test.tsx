import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { DropdownSelect } from "./DropdownSelect";

describe("DropdownSelect", () => {
  afterEach(() => {
    document.body.innerHTML = "";
  });

  it("opens a themed listbox portal and selects an option", async () => {
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const onChange = vi.fn();
    await act(async () => {
      root.render(
        <DropdownSelect
          ariaLabel="정렬 기준"
          value="newest"
          options={[
            { value: "newest", label: "최신 등록순" },
            { value: "score", label: "인기순" },
          ]}
          onChange={onChange}
        />,
      );
    });

    const trigger = container.querySelector<HTMLButtonElement>('[aria-label="정렬 기준"]')!;
    await act(async () => trigger.click());
    const listbox = document.body.querySelector<HTMLElement>('[role="listbox"]');
    expect(listbox).toBeVisible();
    expect(trigger).toHaveAttribute("aria-expanded", "true");

    await act(async () => {
      [...listbox!.querySelectorAll<HTMLButtonElement>('[role="option"]')]
        .find((option) => option.textContent?.includes("인기순"))
        ?.click();
    });
    expect(onChange).toHaveBeenCalledWith("score");
    expect(document.body.querySelector('[role="listbox"]')).toBeNull();

    await act(async () => root.unmount());
  });

  it("supports keyboard navigation and selection", async () => {
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const onChange = vi.fn();
    await act(async () => {
      root.render(
        <DropdownSelect
          ariaLabel="관계"
          value="any"
          options={[
            { value: "any", label: "전체" },
            { value: "pool", label: "Pool 있음" },
          ]}
          onChange={onChange}
        />,
      );
    });

    const trigger = container.querySelector<HTMLButtonElement>('[aria-label="관계"]')!;
    await act(async () => {
      trigger.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true }));
      await new Promise<void>((resolve) => window.requestAnimationFrame(() => resolve()));
    });
    const poolOption = [...document.body.querySelectorAll<HTMLButtonElement>('[role="option"]')]
      .find((option) => option.textContent?.includes("Pool 있음"))!;
    expect(poolOption).toHaveFocus();
    await act(async () => {
      poolOption.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    });
    expect(onChange).toHaveBeenCalledWith("pool");

    await act(async () => root.unmount());
  });
});
