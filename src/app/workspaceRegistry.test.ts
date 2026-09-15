import { describe, expect, expectTypeOf, it } from "vitest";
import { isContentSource, workspaceRegistry, workspaces, type ContentSource, type WorkspaceViewId } from "./workspaceRegistry";

describe("workspace registry", () => {
  it("exposes only the current workspaces and their supported navigation", () => {
    expect(workspaces.map((workspace) => workspace.id)).toEqual(["hitomi", "danbooru", "chzzk"]);
    expect(workspaceRegistry.hitomi.navigation.map((item) => item.view)).toEqual(["explore", "auto-find", "downloads"]);
    expect(workspaceRegistry.danbooru.navigation.map((item) => item.view)).toEqual(["explore", "downloads"]);
    expect(workspaceRegistry.chzzk.navigation.map((item) => item.view)).toEqual(["live", "recordings", "auto-record"]);
    expectTypeOf<ContentSource>().toEqualTypeOf<"hitomi" | "danbooru" | "chzzk">();
    expectTypeOf<WorkspaceViewId<"hitomi">>().toEqualTypeOf<"explore" | "auto-find" | "downloads">();
    expectTypeOf<WorkspaceViewId<"danbooru">>().toEqualTypeOf<"explore" | "downloads">();
  });

  it("recognizes registered own keys without accepting inherited or unknown values", () => {
    for (const workspace of workspaces) {
      expect(isContentSource(workspace.id)).toBe(true);
    }
    for (const value of [null, undefined, 0, {}, "", "unknown", "toString", "constructor", "__proto__"]) {
      expect(isContentSource(value)).toBe(false);
    }
  });
});
