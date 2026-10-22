import { describe, expect, it } from "vitest";
import projection from "../../../src-tauri/src/pinterest_session.js?raw";

type Json = Record<string, unknown>;
const image = { url: "https://i.pinimg.com/originals/aa/bb/cc/example.jpg", width: 400, height: 700 };
const pin = (id = "1234567") => ({ id, title: "Fixture", images: { orig: image }, pinner: { username: "fixture", full_name: "Fixture author" } });
function run(request: Json, initial: Json = {}, extra: { body?: Json; schemas?: Json[]; anchors?: unknown[]; host?: string; frame?: boolean; initialText?: string | null } = {}) {
  const window = { top: null as unknown }; window.top = extra.frame ? {} : window;
  const document = {
    readyState: "complete",
    getElementById: () => ({ textContent: extra.initialText === undefined ? JSON.stringify({ initialReduxState: initial }) : extra.initialText }),
    querySelectorAll: (selector: string) => selector.startsWith("script") ? (extra.schemas ?? []).map(schema => ({ textContent: JSON.stringify(schema) })) : extra.anchors ?? [],
    body: { textContent: JSON.stringify(extra.body ?? {}) },
  };
  return Function("window", "location", "document", `return (\n${projection.replace("__ATSUMI_PINTEREST_REQUEST__", JSON.stringify({ nonce: "fixture-nonce", ...request }))}\n)`)(window, { protocol: "https:", hostname: extra.host ?? "www.pinterest.com", origin: "https://www.pinterest.com", href: "https://www.pinterest.com/" }, document) as Json;
}
describe("Pinterest official document metadata projection", () => {
  it("does not mistake an anonymous homepage shell for an empty successful feed", () => {
    expect(run({ kind: "home" }, { session: { isAuthenticated: false } })).toMatchObject({ nonce: "fixture-nonce", state: "pending", authenticated: false });
  });
  it("returns boolean authentication proof without account identity or secrets", () => {
    const value = run({ kind: "status" }, { session: { isAuthenticated: true, accessToken: "private-value", user: { id: "private-account" } } });
    expect(value).toEqual({ nonce: "fixture-nonce", state: "ready", authenticated: true });
    expect(JSON.stringify(value)).not.toContain("private");
    expect(run({ kind: "status" })).toMatchObject({ authenticated: null });
  });
  it("projects an actually observed home resource and opaque continuation plan", () => {
    const key = JSON.stringify([["page_size", 25], ["field_set_key", "mobile_grid_item"]]);
    const value = run({ kind: "home" }, { session: { isAuthenticated: true }, resources: { HomefeedResource: { [key]: { data: [pin()], nextBookmark: "next-fixture" } } } });
    expect(value).toMatchObject({ state: "ready", authenticated: true, pins: [{ id: "1234567", summaryOnly: true, media: [image] }], continuation: { resource: "HomefeedResource", bookmark: "next-fixture" } });
  });
  it("does not copy unapproved resource options into the continuation plan", () => {
    const key = JSON.stringify([["access_token", "private"]]);
    const value = run({ kind: "home" }, { resources: { HomefeedResource: { [key]: { data: [pin()], nextBookmark: "next" } } } });
    expect(value.continuation).toBeNull();
    expect(JSON.stringify(value)).not.toContain("private");
  });
  it("preserves board-only public profile discovery", () => {
    expect(run({ kind: "profile" }, { resources: { BoardsFeedResource: { fixture: { data: [{ name: "Board", url: "/fixture/board/" }, { name: "foreign", url: "https://evil.test/fixture/board/" }] } } } })).toMatchObject({ state: "ready", pins: [], boards: [{ title: "Board", path: "/fixture/board/" }] });
  });
  it("retains each canonical original page and excludes neighboring schema pins", () => {
    const source = "https://www.pinterest.com/pin/1234567/";
    const value = run({ kind: "pin", id: "1234567" }, {}, { schemas: [
      { "@type": "CreativeWork", url: "https://www.pinterest.com/pin/7777777/", image },
      { "@type": "CreativeWork", mainEntityOfPage: source, image: [image, { ...image, url: image.url.replace("example", "second") }] },
    ] });
    expect(value).toMatchObject({ state: "ready", pins: [{ id: "1234567", summaryOnly: false, media: [image, { ...image, url: image.url.replace("example", "second") }] }] });
    expect(run({ kind: "pin", id: "1234567" }, {}, { schemas: [{ "@type": "CreativeWork", image }] }).state).toBe("unavailable");
  });
  it("never substitutes a recommendation or thumbnail for a canonical original", () => {
    expect(run({ kind: "pin", id: "1234567" }, { pins: { "7654321": pin("7654321") } }).state).toBe("unavailable");
    expect(run({ kind: "pin", id: "1234567" }, { pins: { "1234567": { id: "1234567", images: { "236x": image } } } }).state).toBe("unavailable");
  });
  it("projects actual rendered cards only from official links and does not throw on bad URLs", () => {
    const anchor = (href: string) => ({ href, querySelector: (selector: string) => selector === "img" ? { currentSrc: image.url, naturalWidth: 400, naturalHeight: 700, alt: "Rendered pin" } : null });
    const value = run({ kind: "home" }, {}, { anchors: [anchor("https://["), anchor("https://evil.test/pin/9999999/"), anchor("/pin/1234567/")] });
    expect(value).toMatchObject({ state: "ready", pins: [{ id: "1234567", summaryOnly: true }] });
    expect((value.pins as unknown[])).toHaveLength(1);
  });
  it("distinguishes a real exhausted resource from failure or HTML shells", () => {
    expect(run({ kind: "home", resource: "HomefeedResource", options: {} }, {}, { body: { resource_response: { status: "success", data: [], bookmark: "-end-" } } })).toMatchObject({ state: "ready", pins: [], exhausted: true });
    expect(run({ kind: "home", resource: "HomefeedResource", options: {} }, {}, { body: { resource_response: { status: "failure", data: [] } } }).state).toBe("unavailable");
    expect(run({ kind: "home", resource: "HomefeedResource" }).state).toBe("unavailable");
  });
  it("bounds the result and rejects foreign frames/origins", () => {
    const pins = Array.from({ length: 90 }, (_, i) => pin(String(100000 + i)));
    const value = run({ kind: "home" }, { resources: { HomefeedResource: { fixture: { data: pins } } } });
    expect(value.pins as unknown[]).toHaveLength(60);
    expect(run({ kind: "status" }, {}, { host: "www.pinterest.com.evil.test" }).state).toBe("unavailable");
    expect(run({ kind: "status" }, {}, { frame: true }).state).toBe("unavailable");
  });
});

describe("Pinterest closed diagnostic shapes (no continuation behavior change)", () => {
  const withoutDiagnostic = (value: Json) => { const { diagnostics: _diagnostics, ...product } = value; return product; };
  const diagnostics = (value: Json) => value.diagnostics as Json;
  const anchor = { href: "/pin/1234567/", querySelector: (selector: string) => selector === "img" ? { currentSrc: image.url, naturalWidth: 400, naturalHeight: 700, alt: "Fixture" } : null };
  it("distinguishes structured hydration from DOM-only fallback without recording identities", () => {
    const key = JSON.stringify([["page_size", 25], ["query", "SECRET_QUERY"]]);
    const structured = run({ kind: "home" }, { resources: { HomefeedResource: { [key]: { data: [pin()], nextBookmark: "SECRET_BOOKMARK" } } } });
    expect(diagnostics(structured)).toMatchObject({ sourceBranch: "hydration", initialShape: "redux_object", knownResourceMask: 1, resourceEntryCount: 1, structuredPinCount: 1, domPinCount: 0, dataShape: "array", entryShape: "pin_objects", keyShape: "pairs", bookmarkShape: "string", bookmarkLocationMask: 1, planReject: "none" });
    expect(withoutDiagnostic(structured)).toMatchObject({ state: "ready", exhausted: false, limited: false, continuation: { bookmark: "SECRET_BOOKMARK", options: { query: "SECRET_QUERY" } } });
    const fallback = run({ kind: "keyword" }, {}, { anchors: [anchor] });
    expect(diagnostics(fallback)).toMatchObject({ sourceBranch: "dom_fallback", knownResourceMask: 0, resourceEntryCount: 0, structuredPinCount: 0, domPinCount: 1, planReject: "missing_bookmark" });
    expect(withoutDiagnostic(fallback)).toMatchObject({ state: "ready", exhausted: false, limited: true, continuation: null });
    expect(JSON.stringify(diagnostics(structured))).not.toContain("SECRET");
  });
  it("identifies resource shape failures without guessing a new parsing path", () => {
    const value = run({ kind: "keyword" }, { resources: { BaseSearchResource: { fixture: { data: ["1234567"], nextBookmark: "SECRET_NEXT", options: { bookmarks: ["SECRET_REQUEST"] } } } } }, { anchors: [anchor] });
    expect(diagnostics(value)).toMatchObject({ knownResourceMask: 4, sourceBranch: "dom_fallback", dataShape: "array", entryShape: "id_only", keyShape: "invalid_json", bookmarkShape: "string", bookmarkLocationMask: 5, planReject: "key_shape" });
    expect(value.continuation).toBeNull();
    expect(JSON.stringify(diagnostics(value))).not.toContain("SECRET");
  });
  it.each([
    [null, "absent"], ["", "absent"], ["{", "invalid_json"],
    [JSON.stringify({}), "redux_missing"], ["x".repeat(5 * 1024 * 1024 + 1), "over_limit"],
  ])("projects initial props shape without the original text", (initialText, initialShape) => {
    expect(diagnostics(run({ kind: "home" }, {}, { initialText, anchors: [anchor] }))).toMatchObject({ initialShape, sourceBranch: "dom_fallback" });
  });
  it.each([
    [JSON.stringify([["SECRET_OPTION", "SECRET_VALUE"]]), "SECRET_NEXT", "unknown_option", "string"],
    [JSON.stringify([["page_size", { secret: "SECRET_VALUE" }]]), "SECRET_NEXT", "option_type", "string"],
    [JSON.stringify([["query", "x".repeat(301)]]), "SECRET_NEXT", "option_size", "string"],
    [JSON.stringify(Array.from({ length: 33 }, () => ["gated", false])), "SECRET_NEXT", "too_many_options", "string"],
    [JSON.stringify({ SECRET_OPTION: "SECRET_VALUE" }), "SECRET_NEXT", "key_shape", "string"],
    ["{}", undefined, "missing_bookmark", "absent"],
    ["{}", [], "missing_bookmark", "array"],
  ])("reports a closed rejection category without changing product acceptance", (key, bookmark, planReject, bookmarkShape) => {
    const value = run({ kind: "home" }, { resources: { HomefeedResource: { [key as string]: { data: [pin()], nextBookmark: bookmark } } } });
    expect(diagnostics(value)).toMatchObject({ planReject, bookmarkShape });
    expect(withoutDiagnostic(value)).toMatchObject({ state: "ready", continuation: null, exhausted: false, limited: true });
    expect(JSON.stringify(diagnostics(value))).not.toContain("SECRET");
  });
  it("requires an explicit response terminal marker rather than missing or invalid continuation", () => {
    for (const bookmark of [undefined, ["SECRET_BOOKMARK"]]) {
      const value = run({ kind: "home", resource: "HomefeedResource", options: {} }, {}, { body: { resource_response: { status: "success", data: [pin()], bookmark } } });
      expect(diagnostics(value)).toMatchObject({ sourceBranch: "resource_response", knownResourceMask: 1, resourceEntryCount: 1, structuredPinCount: 1, domPinCount: 0 });
      expect(withoutDiagnostic(value)).toMatchObject({ state: "ready", continuation: null, exhausted: false, limited: true });
    }
    expect(run({ kind: "home", resource: "HomefeedResource", options: {} }, {}, { body: { resource_response: { status: "success", data: [], bookmark: "-end-" } } })).toMatchObject({ state: "ready", exhausted: true, limited: false });
  });
  it("does not copy any account, query, title, identifier, key, bookmark, or option value into diagnostic fields", () => {
    const key = JSON.stringify([["query", "SECRET_QUERY"], ["username", "SECRET_USERNAME"], ["bookmarks", ["SECRET_REQUEST"]]]);
    const value = run({ kind: "keyword" }, {
      session: { isAuthenticated: true, token: "SECRET_TOKEN", user: { id: "SECRET_ACCOUNT" } },
      resources: { BaseSearchResource: { [key]: { data: [{ ...pin("987654321"), title: "SECRET_TITLE" }], nextBookmark: "SECRET_BOOKMARK" } }, SECRET_RESOURCE: "SECRET_EXTRA" },
    });
    const safe = diagnostics(value);
    expect(Object.keys(safe).sort()).toEqual(["sourceBranch", "initialShape", "knownResourceMask", "resourceEntryCount", "structuredPinCount", "domPinCount", "dataShape", "entryShape", "keyShape", "bookmarkShape", "bookmarkLocationMask", "planReject"].sort());
    const encoded = JSON.stringify(safe);
    for (const secret of ["SECRET", "987654321", "BaseSearchResource", "query", "username", "https://"]) expect(encoded).not.toContain(secret);
  });
  it("bounds diagnostic counts even when source resources are large", () => {
    const data = Array.from({ length: 90 }, (_, index) => pin(String(100000 + index)));
    const value = run({ kind: "home" }, { resources: {
      HomefeedResource: { a: { data }, b: { data }, c: { data } },
      UserHomefeedResource: { a: { data }, b: { data }, c: { data } },
    } });
    expect(diagnostics(value)).toMatchObject({ knownResourceMask: 3, resourceEntryCount: 4, structuredPinCount: 60, domPinCount: 0 });
    expect((value.pins as unknown[]).length).toBe(60);
  });
  it("omits optional diagnostics near the unchanged native envelope cap", () => {
    const large = { ...pin(), images: { orig: { ...image, url: "https://i.pinimg.com/originals/" + "x".repeat(512 * 1024 - 1500) + ".jpg" } } };
    const value = run({ kind: "home" }, { resources: { HomefeedResource: { fixture: { data: [large] } } } });
    expect(value.state).toBe("ready");
    expect(value.diagnostics).toBeUndefined();
    expect((value.pins as Json[])[0]?.id).toBe("1234567");
  });
});

describe("Pinterest own saved boards", () => {
  it("projects only an authenticated session-owned path, never a recommended user or secret", () => {
    const source = { session: { isAuthenticated: true, user: { username: "fixture", id: "SECRET_ID", email: "SECRET_EMAIL" } }, resources: { UserResource: { x: { data: { username: "someone_else" } } } } };
    expect(run({ kind: "owner" }, source)).toEqual({ nonce: "fixture-nonce", state: "ready", authenticated: true, ownerPath: "/fixture/_saved/" });
    expect(run({ kind: "status" }, source)).not.toHaveProperty("ownerPath");
    for (const session of [{ isAuthenticated: false, user: { username: "fixture" } }, { isAuthenticated: true }, { isAuthenticated: true, user: { username: "../settings" } }]) {
      expect(run({ kind: "owner" }, { ...source, session }).ownerPath).toBeNull();
    }
  });
  it("projects saved boards and their continuation separately from recommended pins", () => {
    const key = JSON.stringify([["username", "fixture"], ["page_size", 25]]);
    const source = { session: { isAuthenticated: true }, boards: { "7654321": { name: "풍경", url: "/fixture/landscape/" } }, resources: { BoardsFeedResource: { [key]: { data: ["7654321", { name: "unsafe", url: "https://evil.test/fixture/unsafe/" }], nextBookmark: "next" } }, UserPinsResource: { x: { data: [pin()] } } } };
    expect(run({ kind: "saved", owner: "fixture" }, source)).toMatchObject({ state: "ready", pins: [], boards: [{ title: "풍경", path: "/fixture/landscape/" }], continuation: { resource: "BoardsFeedResource", options: { username: "fixture", page_size: 25 }, bookmark: "next" } });
    expect(run({ kind: "saved", owner: "other" }, source).continuation).toBeNull();
    expect(run({ kind: "saved", owner: "fixture" }, { resources: { UserPinsResource: { x: { data: [pin()] } } } }).state).toBe("pending");
  });
  it("distinguishes an empty saved collection from an unrecognized document", () => {
    const value = run({ kind: "saved", owner: "fixture", resource: "BoardsFeedResource", options: { username: "fixture" } }, {}, { body: { resource_response: { status: "success", data: [], bookmark: "-end-" } } });
    expect(value).toMatchObject({ state: "ready", pins: [], boards: [], exhausted: true });
    expect(run({ kind: "saved", owner: "fixture" }).state).toBe("pending");
  });
});

describe("Pinterest observed-resource continuation candidate", () => {
  const feed = (resource: string, pairs: unknown[], bookmark: unknown = "next-page") => ({
    session: { isAuthenticated: true },
    resources: { [resource]: { [JSON.stringify(pairs)]: { data: [pin()], nextBookmark: bookmark } } },
  });
  it("drops unknown fields only for the two observed kind/resource candidates", () => {
    const pairs = [["page_size", 25], ["unknown_fixture_option", { SECRET: "SECRET_VALUE" }]];
    const home = run({ kind: "home", input: "" }, feed("UserHomefeedResource", pairs));
    expect(home.continuation).toEqual({ resource: "UserHomefeedResource", options: { page_size: 25 }, bookmark: "next-page" });
    expect(JSON.stringify(home.continuation)).not.toContain("SECRET");
    expect(JSON.stringify(home.diagnostics)).not.toContain("unknown_fixture_option");
    const keyword = run({ kind: "keyword", input: " landscape " }, feed("BaseSearchResource", [...pairs, ["query", "landscape"], ["scope", "pins"]]));
    expect(keyword.continuation).toEqual({ resource: "BaseSearchResource", options: { page_size: 25, query: "landscape", scope: "pins" }, bookmark: "next-page" });
    for (const [kind, resource] of [["home", "HomefeedResource"], ["profile", "UserPinsResource"], ["profile", "BoardFeedResource"]]) {
      expect(run({ kind }, feed(resource!, pairs)).continuation).toBeNull();
    }
  });
  it.each([
    [["unknown_fixture_option", true]],
    [["page_size", 25], ["page_size", 30]],
    [["page_size", 25], ["unknown_fixture_option", true], ["unknown_fixture_option", false]],
    [["page_size", { invalid: true }], ["unknown_fixture_option", true]],
    [["page_size", 25], ["malformed"]],
    [["page_size", 25], [123, "not-a-key"]],
    [["page_size", 25], ["access_token", "SECRET_TOKEN"]],
  ])("keeps empty, duplicate, malformed, invalid-known and credential plans rejected", (...pairs) => {
    expect(run({ kind: "home", input: "" }, feed("UserHomefeedResource", pairs)).continuation).toBeNull();
  });
  it.each([
    [["page_size", 25]],
    [["query", "other"], ["scope", "pins"]],
    [["query", "landscape"], ["scope", "boards"]],
    [["query", "landscape"]],
  ])("never invents missing keyword context or accepts a different scope", (...pairs) => {
    const value = run({ kind: "keyword", input: "landscape" }, feed("BaseSearchResource", pairs));
    expect(value.continuation).toBeNull();
    expect(value.diagnostics).toMatchObject({ planReject: "context_mismatch" });
  });
  it("does not broaden resource selection across request kinds", () => {
    expect(run({ kind: "home", input: "" }, feed("BaseSearchResource", [["query", "landscape"], ["scope", "pins"]])).state).toBe("pending");
    expect(run({ kind: "keyword", input: "landscape" }, feed("UserHomefeedResource", [["page_size", 25]])).state).toBe("pending");
  });
  it("marks initial hydration terminal only with explicit end and no conflicting candidate", () => {
    expect(run({ kind: "home", input: "" }, feed("UserHomefeedResource", [["page_size", 25]], "-end-"))).toMatchObject({ state: "ready", exhausted: true, limited: false, continuation: null });
    const value = feed("UserHomefeedResource", [["page_size", 25]], "-end-");
    const resources = value.resources.UserHomefeedResource!;
    resources[JSON.stringify([["page_size", 26]])] = { data: [pin()], nextBookmark: undefined };
    expect(run({ kind: "home", input: "" }, value)).toMatchObject({ exhausted: false, limited: true });
  });
  it("does not use echo-only resource options as next-page or terminal evidence", () => {
    const request = { kind: "home", input: "", resource: "UserHomefeedResource", options: { page_size: 25 }, bookmark: "current-page" };
    for (const echo of ["current-page", "-end-", "different-unproven-option-value"]) {
      const value = run(request, {}, { body: { resource_response: { status: "success", data: [] }, resource: { options: { bookmarks: [echo] } } } });
      expect(value).toMatchObject({ state: "ready", pins: [], exhausted: false, limited: true, continuation: null });
    }
    const same = run(request, {}, { body: { resource_response: { status: "success", data: [], bookmark: "current-page" } } });
    expect(same).toMatchObject({ state: "ready", exhausted: false, limited: true, continuation: null, diagnostics: { planReject: "same_bookmark" } });
    const next = run(request, {}, { body: { resource_response: { status: "success", data: [], bookmark: "next-page" } } });
    expect(next).toMatchObject({ state: "ready", exhausted: false, limited: false, continuation: { bookmark: "next-page" } });
  });
  it("keeps first empty documents pending and valid empty incomplete follow-ups limited", () => {
    expect(run({ kind: "home", input: "" })).toMatchObject({ state: "pending" });
    const value = run({ kind: "home", input: "", resource: "UserHomefeedResource", options: { page_size: 25 }, bookmark: "current" }, {}, { body: { resource_response: { status: "success", data: [] } } });
    expect(value).toMatchObject({ state: "ready", pins: [], exhausted: false, limited: true, continuation: null });
  });
});
