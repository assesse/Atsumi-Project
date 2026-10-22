import { describe, expect, expectTypeOf, it } from "vitest";
import { hasCommunity, isContentSource, workspaceRegistry, workspaces, type ContentSource, type WorkspaceViewId } from "./workspaceRegistry";

describe("workspace registry", () => {
  it("exposes only the current workspaces and their supported navigation", () => {
    expect(workspaces.map((workspace) => workspace.id)).toEqual(["hitomi", "danbooru", "chzzk", "youtube", "soop", "pinterest", "pixiv", "x"]);
    expect(workspaceRegistry.hitomi.navigation.map((item) => item.view)).toEqual(["explore", "auto-find", "downloads"]);
    expect(workspaceRegistry.danbooru.navigation.map((item) => item.view)).toEqual(["explore", "downloads"]);
    expect(workspaceRegistry.chzzk.navigation.map((item) => item.view)).toEqual(["live", "screenshots", "recordings"]);
    expectTypeOf<ContentSource>().toEqualTypeOf<"hitomi" | "danbooru" | "chzzk" | "youtube" | "soop" | "pinterest" | "pixiv" | "x">();
    expect(workspaceRegistry.x.navigation.map(item=>item.view)).toEqual(["explore"]);
    expectTypeOf<WorkspaceViewId<"hitomi">>().toEqualTypeOf<"explore" | "auto-find" | "downloads">();
    expectTypeOf<WorkspaceViewId<"danbooru">>().toEqualTypeOf<"explore" | "downloads">();
    expectTypeOf<WorkspaceViewId<"chzzk">>().toEqualTypeOf<"live" | "screenshots" | "recordings">();
    for (const source of ["youtube", "soop"] as const) {
      expect(workspaceRegistry[source].navigation.map(item => item.view)).toEqual(["live", "screenshots", "downloads", "explore"]);
      expect(workspaceRegistry[source].navigation.slice(0, 3).map(({ label, icon }) => ({ label, icon }))).toEqual(workspaceRegistry.chzzk.navigation.map(({ label, icon }) => ({ label, icon })));
    }
    expectTypeOf<WorkspaceViewId<"soop">>().toEqualTypeOf<"explore" | "live" | "screenshots" | "downloads">();
    for (const source of ["pinterest", "pixiv"] as const) expect(workspaceRegistry[source].navigation.map(item => item.view)).toEqual(["explore", "favorites", "downloads"]);
    expect(workspaces.filter(workspace => hasCommunity(workspace.id)).map(workspace => workspace.id)).toEqual(["hitomi", "danbooru"]);
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
