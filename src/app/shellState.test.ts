import { describe, expect, it } from "vitest";
import { galleryId } from "../core/types";
import { initialUiState, uiReducer } from "../state/uiState";
import { createShellState, shellReducer } from "./shellState";

describe("shell and gallery ownership", () => {
  it("keeps application chrome when switching workspaces", () => {
    const initial = createShellState("hitomi");
    const collapsed = shellReducer(initial, { type: "rail.toggle" });
    const settings = shellReducer(collapsed, { type: "settings.set", open: true });
    const activity = shellReducer(settings, { type: "activity.set", open: true });
    const switched = shellReducer(activity, { type: "source.select", source: "danbooru" });
    expect(switched).toEqual({ source: "danbooru", railCollapsed: true, settingsOpen: true, activityOpen: true });
    expect(initial).toEqual(createShellState("hitomi"));
    expect(shellReducer(switched, { type: "source.select", source: "danbooru" })).toBe(switched);
  });

  it("does not store app overlays in the gallery reducer or gallery data in the shell", () => {
    const gallery = uiReducer(initialUiState, { type: "detail.open", id: galleryId(123) });
    expect(gallery).not.toHaveProperty("railCollapsed");
    expect(gallery.overlays).toEqual({ reviewGalleryId: null });
    expect(createShellState("hitomi")).not.toHaveProperty("detail");
    expect(createShellState("hitomi")).not.toHaveProperty("search");
    const switched = shellReducer(createShellState("hitomi"), { type: "source.select", source: "danbooru" });
    expect(switched.source).toBe("danbooru");
    expect(gallery.detail.tabs).toEqual([galleryId(123)]);
  });
});
