import { describe, expect, it } from "vitest";
import source from "../../../src-tauri/src/soop/discovery_session.js?raw";
const vmName = "node:vm";
const { runInNewContext } = await import(vmName) as { runInNewContext(source: string, context: Record<string, unknown>): any };

const script = source.replace("__ATSUMI_SOOP_DISCOVERY_NONCE__", '"test-nonce"');
const endpoint = "https://live.sooplive.com/api/myplus/preferbjLiveVodController.php";
function run(value: unknown, href = endpoint, readyState = "complete") {
  const window: { top?: unknown } = {}; window.top = window;
  return runInNewContext(script, { window, location: { href }, document: { readyState, body: { textContent: JSON.stringify(value) } } });
}
describe("SOOP browser-owned home projection", () => {
  it("projects only bounded public rows, not private fields", () => {
    const value = run({ RESULT: 1, DATA: { LIVE_LIST: [{ USER_ID: "channel", USER_NICK: "public name", BROAD_NO: "123", BROAD_TITLE: "public title", secret: "never-export", cookie: "never-export" }], VOD_LIST: [] }, account: "never-export" });
    expect(value.state).toBe("ready"); expect(value.nonce).toBe("test-nonce"); expect(value.live[0].user_id).toBe("channel");
    expect(JSON.stringify(value)).not.toContain("never-export");
  });
  it("accepts official transformed case but caps rows and field lengths", () => {
    const value = run({ result: 1, data: { liveList: [], vodList: Array.from({ length: 99 }, () => ({ bjId: "a".repeat(500), titleName: "b".repeat(900), titleNo: "123" })) } });
    expect(value.video).toHaveLength(40); expect(value.video[0].bj_id).toHaveLength(40); expect(value.video[0].title_name).toHaveLength(300);
  });
  it("uses the official preferred-card fields without substituting a duration for an ID", () => {
    const value = run({ result: 1, data: { liveList: [], vodList: [{ userId: "creator", userNick: "channel name", titleNo: "209244227", title: "actual VOD title", thumbnail: "https://videoimg.sooplive.com/php/SnapshotLoad.php?rowKey=public", duration: "11109467", vodDuration: "3:05:09", ppv: true, authNo: 1, account: "never-export" }] } });
    expect(value.video[0]).toMatchObject({ user_id: "creator", title_no: "209244227", title: "actual VOD title", vod_duration: "3:05:09", ppv: true, thumbnail: expect.stringContaining("SnapshotLoad.php") });
    expect(value.video[0].title_no).not.toBe(value.video[0].duration);
    expect(JSON.stringify(value)).not.toContain("never-export"); expect(value.video[0]).not.toHaveProperty("authNo");
  });
  it("bounds additional public fields and preserves official original-case aliases", () => {
    const value = run({ RESULT: 1, DATA: { LIVE_LIST: [], VOD_LIST: [{ USER_ID: "creator", TITLE_NO: 209244227, TITLE: "t".repeat(1000), THUMBNAIL: "u".repeat(3000), VOD_DURATION: "0:09", PPV: "Y" }] } });
    expect(value.video[0].title).toHaveLength(300); expect(value.video[0].thumbnail).toHaveLength(2048);
    expect(value.video[0].vod_duration).toBe("0:09"); expect(value.video[0].ppv).toBe(true);
  });
  it("distinguishes pending, unavailable, malformed and a valid empty recommendation", () => {
    expect(run({}, endpoint, "loading").state).toBe("pending");
    expect(run({ RESULT: -1, DATA: [] }).state).toBe("unavailable");
    expect(run({ RESULT: 1, DATA: [] }).state).toBe("unavailable");
    expect(run({ RESULT: 1, DATA: { LIVE_LIST: [], VOD_LIST: [] } }).state).toBe("ready");
    expect(run({ RESULT: 1, DATA: {} }, "https://evil.test").state).toBe("unavailable");
    expect(run({ huge: "x".repeat(2 * 1024 * 1024) }).state).toBe("unavailable");
  });
  it("projects bounded public tag arrays without accepting objects or control characters", () => {
    const value = run({ RESULT: 1, DATA: { LIVE_LIST: [{ USER_ID: "creator", BROAD_NO: 123, LANG_TAGS: ["한국어"], CATEGORY_TAGS: ["버추얼"], HASH_TAGS: ["공개 태그", { private: "never-export" }, "bad\nvalue", 7, "x".repeat(100)] }], VOD_LIST: [] } });
    expect([...value.live[0].lang_tags]).toEqual(["한국어"]);
    expect([...value.live[0].category_tags]).toEqual(["버추얼"]);
    expect([...value.live[0].hash_tags]).toEqual(["공개 태그", "x".repeat(32)]);
    expect(JSON.stringify(value)).not.toContain("never-export");
  });
});
