import { describe, expect, it } from "vitest";
import script from "../../../src-tauri/examples/soop_embed_probe.js?raw";
import harness from "../../../src-tauri/examples/soop_embed_probe.rs?raw";

const vmName = "node:vm";
const { runInNewContext } = await import(vmName) as { runInNewContext(source: string, context: Record<string, unknown>): unknown };
type Track = { mime: string; appends: number; bytes: number; changes: number; containers: string[] };
type ProbeState = {
  ready: string;
  videos: Array<{ id: string; ad: boolean; sourceBound: boolean; tracks: Track[]; encrypted: boolean; srcObject: boolean }>;
  mse: { available: boolean; hooked: boolean; sources: number; buffers: number; overflow: boolean };
  chat: { visible: boolean; inputPresent: boolean; inputEnabled: boolean };
  forcedPlay: boolean; agentAbsenceVerified: boolean;
};
type ProbeMessage = { type: string; state: ProbeState };

function child(options: { url?: string; nested?: boolean; noMse?: boolean } = {}) {
  const calls: unknown[] = [];
  class SourceBuffer {
    failure: Error | null = null;
    appendBuffer(value: unknown) { if (this.failure) throw this.failure; calls.push(value); return "original-append"; }
    changeType(_type: string) { if (this.failure) throw this.failure; return "original-change"; }
  }
  class MediaSource {
    addSourceBuffer(_mime: string) { return new SourceBuffer(); }
  }
  let urlCount = 0;
  class Url {
    static createObjectURL(_value: unknown) { return `blob:must-not-be-exported-${urlCount++}`; }
  }
  const originalAppend = SourceBuffer.prototype.appendBuffer;
  const originalCreate = Url.createObjectURL;
  const messages: ProbeMessage[] = [];
  const parent = { postMessage: (message: ProbeMessage, target: string) => { expect(target).toBe("http://tauri.localhost"); messages.push(message); } };
  const input = { disabled: false, getBoundingClientRect: () => ({ width: 200, height: 40 }), get value(): never { throw new Error("No input reads"); } };
  const chat = { hidden: false, getBoundingClientRect: () => ({ width: 300, height: 500 }), querySelector: () => input, get innerText(): never { throw new Error("No chat text reads"); } };
  const video = { id: "livePlayer", videoWidth: 1920, videoHeight: 1080, currentTime: 4, readyState: 4,
    paused: false, ended: false, error: null, currentSrc: "", src: "", mediaKeys: null as object | null, srcObject: null as object | null,
    getBoundingClientRect: () => ({ width: 960, height: 540 }), getVideoPlaybackQuality: () => ({ totalVideoFrames: 120 }), closest: () => null };
  const location = new URL(options.url ?? "https://play.sooplive.com/fixture/embed");
  const window = { MediaSource: options.noMse ? undefined : MediaSource, SourceBuffer, URL: Url, top: options.nested ? {} : parent, parent };
  const document = { readyState: "complete", querySelectorAll: () => [video], getElementById: () => chat,
    get cookie(): never { throw new Error("No cookies"); }, get title(): never { throw new Error("No page text"); } };
  let timer: (() => void) | null = null;
  let cleared = false;
  runInNewContext(script, { window, document, location, parent, Uint8Array, ArrayBuffer,
    getComputedStyle: (element: unknown) => ({ display: element === chat && chat.hidden ? "none" : "block", visibility: "visible" }),
    setInterval: (callback: () => void) => { timer = callback; return 1; }, clearInterval: () => { cleared = true; } });
  const source = new MediaSource();
  video.currentSrc = video.src = Url.createObjectURL(source);
  const buffer = source.addSourceBuffer('video/mp4; codecs="avc1.42E01E"');
  return { window, video, chat, input, source, buffer, MediaSource, SourceBuffer, Url, calls, messages, originalAppend, originalCreate,
    sample: () => { timer?.(); return messages.at(-1)?.state; }, active: () => timer !== null, cleared: () => cleared };
}
const box = (kind: string) => { const bytes = new Uint8Array(12); new DataView(bytes.buffer).setUint32(0, 12); for (let i = 0; i < 4; i++) bytes[4 + i] = kind.charCodeAt(i); return bytes; };

describe("isolated SOOP original-player probe", () => {
  it("observes only accepted original MSE appends and binds the actual video source", () => {
    const f = child();
    const bytes = box("ftyp");
    expect(f.buffer.appendBuffer(bytes)).toBe("original-append");
    expect(f.calls[0]).toBe(bytes);
    f.buffer.appendBuffer(box("moof"));
    const state = f.sample()!;
    expect(state.mse).toMatchObject({ available: true, hooked: true, sources: 1, buffers: 1, overflow: false });
    expect(state.videos[0]).toMatchObject({ id: "livePlayer", sourceBound: true, ad: false,
      tracks: [{ mime: 'video/mp4; codecs="avc1.42E01E"', appends: 2, bytes: 24, changes: 0, containers: ["ftyp", "moof"] }] });
    expect(state.forcedPlay).toBe(false);
    expect(state.agentAbsenceVerified).toBe(false);
    f.video.currentSrc = "blob:other-source";
    expect(f.sample()!.videos[0]!.sourceBound).toBe(false);
  });
  it("preserves native errors, input slice identity and current codec changes", () => {
    const f = child(); const error = new Error("native quota");
    f.buffer.failure = error;
    expect(() => f.buffer.appendBuffer(box("ftyp"))).toThrow(error);
    expect(f.sample()!.videos[0]!.tracks[0]!.appends).toBe(0);
    f.buffer.failure = null;
    const storage = new Uint8Array(40); storage.set(box("moof"), 9);
    const slice = storage.subarray(9, 21);
    expect(f.buffer.appendBuffer(slice)).toBe("original-append");
    expect(f.calls[0]).toBe(slice);
    expect(f.buffer.changeType('video/mp4; codecs="hvc1.1.6.L93.B0"')).toBe("original-change");
    expect(f.sample()!.videos[0]!.tracks[0]).toMatchObject({ changes: 1, mime: 'video/mp4; codecs="hvc1.1.6.L93.B0"', containers: ["moof"] });
  });
  it.each(["https://evil.test/fixture/embed", "http://play.sooplive.com/fixture/embed", "https://play.sooplive.com/fixture", "https://play.sooplive.com/other/fixture/embed"])("does not install in %s", (url) => {
    const f = child({ url }); expect(f.active()).toBe(false); expect(f.SourceBuffer.prototype.appendBuffer).toBe(f.originalAppend);
  });
  it("does not install into nested third-party frames and reports missing MSE honestly", () => {
    const nested = child({ nested: true }); expect(nested.active()).toBe(false);
    const f = child({ noMse: true }); expect(f.sample()!.mse).toMatchObject({ available: false, hooked: false });
  });
  it("reads only chat geometry and disabled state, never private text or account data", () => {
    const f = child(); const state = f.sample()!;
    expect(state.chat).toMatchObject({ visible: true, inputPresent: true, inputEnabled: true });
    f.input.disabled = true; f.chat.hidden = true;
    expect(f.sample()!.chat).toMatchObject({ visible: false, inputPresent: true, inputEnabled: false });
    expect(JSON.stringify(state)).not.toContain("blob:");
    for (const forbidden of ["document.cookie", "localStorage", ".innerText", ".textContent", "fetch(", "XMLHttpRequest", ".play(", ".focus(", "MediaRecorder", "captureStream("]) expect(script).not.toContain(forbidden);
  });
  it("bounds sources and buffers, then restores only its own native hooks", () => {
    const f = child();
    for (let i = 0; i < 20; i++) { const source = new f.MediaSource(); f.Url.createObjectURL(source); for (let j = 0; j < 10; j++) source.addSourceBuffer("video/mp4"); }
    expect(f.sample()!.mse).toMatchObject({ sources: 8, buffers: 16, overflow: true });
    for (let i = 0; i < 138; i++) f.sample();
    expect(f.cleared()).toBe(true);
    expect(f.SourceBuffer.prototype.appendBuffer).toBe(f.originalAppend);
    expect(f.Url.createObjectURL).toBe(f.originalCreate);
  });
  it("the host accepts only its exact frame and strips arbitrary child-supplied fields", () => {
    const listeners = new Map<string, (event: Record<string, unknown>) => void>();
    const contentWindow = {};
    const window: { top?: unknown; soopProbe?: { samples: number; frame: ProbeState } } = {};
    window.top = window;
    runInNewContext(script, { window, location: new URL("http://tauri.localhost/soop-probe.html"),
      document: { getElementById: () => ({ contentWindow }) }, addEventListener: (name: string, cb: (event: Record<string, unknown>) => void) => { listeners.set(name, cb); } });
    const event = { origin: "https://play.sooplive.com", source: contentWindow, data: { type: "atsumi-soop-probe", state: {
      title: "private title", messages: ["private chat"], ticket: "private ticket", videos: [{ id: "livePlayer", currentSrc: "https://private.invalid", tracks: [{ mime: "https://private.invalid", containers: ["secret", "ftyp"] }] }],
      chat: { visible: true, inputValue: "private input" }, ready: "complete", forcedPlay: true, agentAbsenceVerified: true,
    } } };
    listeners.get("message")!({ ...event, origin: "https://evil.test" });
    listeners.get("message")!({ ...event, source: {} });
    expect(window.soopProbe!.samples).toBe(0);
    listeners.get("message")!(event);
    expect(window.soopProbe!.samples).toBe(1);
    expect(window.soopProbe!.frame.videos[0]!.tracks[0]).toMatchObject({ mime: "unrecognized", containers: ["ftyp"] });
    expect(JSON.stringify(window.soopProbe)).not.toMatch(/private|secret|ticket|currentSrc/);
    expect(window.soopProbe!.frame.forcedPlay).toBe(false);
    expect(window.soopProbe!.frame.agentAbsenceVerified).toBe(false);
  });
  it("keeps parent and child unfocused, muted and isolated without claiming capture success", () => {
    expect(harness.match(/\.focused\(false\)/g)).toHaveLength(2);
    expect(harness).toContain("--mute-audio");
    expect(harness).toContain('.prefix("atsumi-soop-embed-")');
    expect(harness).toContain('"captureValidated":false');
    expect(harness).toContain('"externalAgentAbsenceVerified":false');
    expect(harness).not.toContain("SOOP_EMBED_PROBE_EVAL: {error}");
  });
});
