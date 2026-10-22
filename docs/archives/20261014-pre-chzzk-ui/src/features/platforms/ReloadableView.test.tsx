import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { ReloadableView } from "./ReloadableView";

it("retries a rejected import with a fresh lazy component and preserves props", async () => {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const load = vi.fn<() => Promise<{ default: (props: { title: string }) => React.JSX.Element }>>()
    .mockRejectedValueOnce(new Error("temporary chunk failure"))
    .mockResolvedValue({ default: ({ title }) => <h1>{title}</h1> });
  const log = vi.spyOn(console, "error").mockImplementation(() => {});
  try {
    await act(async () => root.render(<ReloadableView load={load} viewProps={{ title: "YouTube" }} />));
    expect(container.querySelector('[role="alert"]')).toHaveTextContent("화면을 열지 못했습니다.");
    expect(load).toHaveBeenCalledTimes(1);
    await act(async () => container.querySelector<HTMLButtonElement>("button")!.click());
    expect(load).toHaveBeenCalledTimes(2);
    expect(container.querySelector("h1")).toHaveTextContent("YouTube");
  } finally {
    await act(async () => root.unmount());
    container.remove();
    log.mockRestore();
  }
});
