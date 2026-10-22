import { act, useCallback, useState, type ComponentProps } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { createMultiviewApi, emptyMultiview, multiviewEntries, normalizeMultiviewChannel, type MultiviewApi, type MultiviewSnapshot } from "../../api/multiview";
import { MadoWorkspace, NativeStreamingPane, readMadoLayout, removeClosedViewer, projectControlSnapshot } from "./MadoWorkspace";
import { createOfficialBrowserApi, emptyOfficialBrowserSnapshot } from "../../api/officialBrowser";
import { tutorialStepEvent } from "../../tutorial/tourActions";
import type { WindowFullscreenGrant, WindowFullscreenOutcome } from "../../api/windowFullscreen";
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
const nativeWindow = vi.hoisted(() => ({ isMinimized: vi.fn(), isVisible: vi.fn(), isFullscreen: vi.fn(), setFullscreen: vi.fn() }));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => nativeWindow }));
const presentationEvents = vi.hoisted(() => new Set<(event: { payload: { action: string; channelId: string } }) => void>());
const fullscreenLostEvents = vi.hoisted(() => new Set<(event: { payload: WindowFullscreenGrant }) => void>());
const viewerClosedEvents = vi.hoisted(() => new Set<(event: { payload: { snapshot: MultiviewSnapshot; channelId: string } }) => void>());
const windowFullscreen = vi.hoisted(() => ({ acquire: vi.fn(), release: vi.fn(), current: vi.fn() }));
vi.mock("../../api/windowFullscreen", () => ({ windowFullscreenApi: windowFullscreen, sameFullscreenGrant: (a: WindowFullscreenGrant | null, b: WindowFullscreenGrant | null) => !!a && !!b && a.owner === b.owner && a.documentEpoch === b.documentEpoch && a.revision === b.revision && a.windowKey === b.windowKey }));
vi.mock("@tauri-apps/api/event", () => ({ listen: async (name: string, handler: (event: { payload: { action: string; channelId: string } }) => void) => {
  if (name === 'chzzk-presentation') presentationEvents.add(handler);
  const lostHandler = handler as unknown as (event: { payload: WindowFullscreenGrant }) => void;
  if (name === 'platform-window-fullscreen:lost') fullscreenLostEvents.add(lostHandler);
  const closedHandler=handler as unknown as (event:{payload:{snapshot:MultiviewSnapshot;channelId:string}})=>void;
  if(name==='chzzk-viewer-closed')viewerClosedEvents.add(closedHandler);
  return () => { presentationEvents.delete(handler); fullscreenLostEvents.delete(lostHandler); viewerClosedEvents.delete(closedHandler); };
} }));
// Simulate the parent's sidebar request; the live workspace has no own header.
function WorkspaceFixture(props: ComponentProps<typeof MadoWorkspace>) {
  const [request, setRequest] = useState(0);
  const handled = useCallback(() => setRequest(0), []);
  return <>{props.unifiedLive ? <aside><button type="button" onClick={() => setRequest(value => value + 1)}>방송 보기</button></aside> : null}<MadoWorkspace {...props} connectionRequest={request} onConnectionRequestHandled={handled} /></>;
}
const A = "a".repeat(32), B = "b".repeat(32), C = "c".repeat(32), D = "d".repeat(32);
const success = <T,>(data: T) => ({ ok: true as const, data });
const multiple = (oneVideo = false): MultiviewSnapshot => ({ active: true, epoch: 7, audioOwner: null, panes: [A, B, C, D].flatMap((channelId, index) => [
  ...(!oneVideo || index === 0 ? [{ paneId: `video-${index}`, channelId, channelName: `도서관 ${index + 1}`, kind: "video" as const, status: "ready", ready: true, audioEnabled: false }] : []),
  { paneId: `chat-${index}`, channelId, channelName: `도서관 ${index + 1}`, kind: "chat" as const, status: "ready" },
]) });
const deferred = <T,>() => { let resolve!: (value: T) => void; const promise = new Promise<T>((done) => { resolve = done; }); return { promise, resolve }; };
const active = (): MultiviewSnapshot => ({ active: true, epoch: 7, audioOwner: null, panes: [
  { paneId: "mado-7-a-video", channelId: A, kind: "video", status: "ready", ready: true, audioEnabled: false },
  { paneId: "mado-7-a-chat", channelId: A, kind: "chat", status: "ready" },
] });
const fake = () => ({ runtime: "tauri" as const, configure: vi.fn<MultiviewApi["configure"]>().mockResolvedValue(success(active())),
  snapshot: vi.fn<MultiviewApi["snapshot"]>().mockResolvedValue(success(emptyMultiview())),
  close: vi.fn<MultiviewApi["close"]>().mockResolvedValue(success(emptyMultiview())),
  setAudio: vi.fn<MultiviewApi["setAudio"]>().mockResolvedValue(success(active())),
  setPaneAudio: vi.fn<MultiviewApi["setPaneAudio"]>().mockResolvedValue(success(active())),
  requestControl: vi.fn<MultiviewApi["requestControl"]>().mockResolvedValue(success(active())),
  confirmControl: vi.fn<MultiviewApi["confirmControl"]>().mockResolvedValue(success(active())),
  ackUiAction: vi.fn<MultiviewApi["ackUiAction"]>().mockResolvedValue(success(active())),
  setViewport: vi.fn<MultiviewApi["setViewport"]>().mockResolvedValue(success(undefined)),
});
let container: HTMLDivElement, root: Root;
beforeEach(() => { localStorage.clear(); vi.mocked(invoke).mockResolvedValue(success(emptyOfficialBrowserSnapshot("tauri"))); nativeWindow.isMinimized.mockResolvedValue(false); nativeWindow.isVisible.mockResolvedValue(true); nativeWindow.isFullscreen.mockResolvedValue(false); nativeWindow.setFullscreen.mockResolvedValue(undefined);
  let revision = 0;
  windowFullscreen.acquire.mockReset().mockImplementation(async (owner: string, documentEpoch: number) => ({ grant: { owner, documentEpoch, revision: ++revision, windowKey: 'main-test' }, error: null }));
  windowFullscreen.release.mockReset().mockResolvedValue(true); windowFullscreen.current.mockReset().mockResolvedValue(true);
});
beforeEach(() => { vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true); vi.useFakeTimers(); vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => setTimeout(() => cb(0), 16)); vi.stubGlobal("cancelAnimationFrame", clearTimeout); container = document.createElement("div"); document.body.append(container); root = createRoot(container); });
afterEach(async () => { await act(async () => root.unmount()); container.remove(); vi.useRealTimers(); vi.unstubAllGlobals(); vi.restoreAllMocks(); });
const button = (text: string) => [...container.querySelectorAll<HTMLButtonElement>("button")].find((element) => element.textContent === text || element.getAttribute('aria-label') === text)!;
const enter = async (index: number, text: string) => act(async () => {
  const input = container.querySelector<HTMLInputElement>(`#mado-channel-${index}`)!;
  Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, text); input.dispatchEvent(new Event("input", { bubbles: true }));
});
describe('single-viewer close ordering', () => {
  it('projects only exact surviving control panes while preserving their fresh status and controls',()=>{
    const initial=multiple(),current=removeClosedViewer(initial,{channelId:A,snapshot:initial});
    const reply={...initial,audioOwner:A,pendingUiAction:{paneId:'video-0',id:'old',action:'exit_focus' as const},pendingControl:{paneId:'video-1',channelId:B,id:'survivor',action:'screenshot' as const,expiresAt:100},panes:initial.panes.map(pane=>pane.paneId==='video-1'?{...pane,recordingId:'still-recording',chatCount:23}:pane)};
    const next=projectControlSnapshot(current,reply);
    expect(next.panes.map(pane=>pane.channelId)).toEqual([B,B,C,C,D,D]);
    expect(next.panes[0]).toMatchObject({recordingId:'still-recording',chatCount:23});
    expect(next.pendingControl).toBe(reply.pendingControl);expect(next.pendingUiAction).toBeNull();expect(next.audioOwner).toBeNull();
  });
  it('rejects different epochs, closed layouts, and reused pane ids with different source or kind',()=>{
    const current=active();expect(projectControlSnapshot(current,{...current,epoch:8})).toBe(current);
    const closed=removeClosedViewer(current,{channelId:A,snapshot:current});expect(projectControlSnapshot(closed,current)).toBe(closed);
    for(const patch of [{channelId:B},{kind:'chat' as const}]){
      const reply={...current,panes:[{...current.panes[0]!,...patch}],pendingControl:{paneId:current.panes[0]!.paneId,channelId:B,id:'wrong',action:'screenshot' as const,expiresAt:100}};
      expect(projectControlSnapshot(current,reply)).toMatchObject({active:false,panes:[],pendingControl:null});
    }
  });
  it.each(['request','confirm'] as const)('does not resurrect viewers from a late %s response and releases pending state', async phase => {
    const api=fake(), state=multiple(true), waiting=deferred<Awaited<ReturnType<MultiviewApi['requestControl']>>>();
    state.pendingControl={paneId:'video-0',id:'late-control',action:'screenshot',channelId:A,expiresAt:Date.now()+20000};
    api.snapshot.mockResolvedValue(success(phase==='request'?{...state,pendingControl:null}:state));
    if(phase==='request')api.requestControl.mockReturnValue(waiting.promise);else api.confirmControl.mockReturnValue(waiting.promise);
    await act(async()=>root.render(<WorkspaceFixture runtime="tauri" privacy={false} onLeave={()=>{}} api={api}/>));
    if(phase==='request')await act(async()=>window.dispatchEvent(new KeyboardEvent('keydown',{key:'s',bubbles:true})));
    else await act(async()=>button('취소').click());
    expect(phase==='request'?api.requestControl:api.confirmControl).toHaveBeenCalledOnce();
    await act(async()=>{for(const channelId of [B,A])for(const handler of viewerClosedEvents)handler({payload:{channelId,snapshot:state}});});
    expect(container.querySelectorAll('.mado-native-slot')).toHaveLength(2);
    await act(async()=>waiting.resolve(success(state)));
    expect(container.querySelectorAll('.mado-native-slot')).toHaveLength(2);
    expect(container.querySelector('[role="alertdialog"]')).toBeNull();
    await act(async()=>button('연결 설정').click());
    expect(button('일반모드')).toBeEnabled();
    expect(api.close).not.toHaveBeenCalled(); expect(api.configure).not.toHaveBeenCalled();
  });
  it('does not resurrect a channel when same-epoch close events arrive backwards', () => {
    const initial = multiple();
    const afterA = removeClosedViewer(initial, { channelId: A, snapshot: initial });
    const afterB = removeClosedViewer(afterA, { channelId: B, snapshot: afterA });
    const reversedB = removeClosedViewer(initial, { channelId: B, snapshot: afterB });
    const reversedA = removeClosedViewer(reversedB, { channelId: A, snapshot: afterA });
    expect(reversedA.panes).toEqual(afterB.panes);
    expect(reversedA.panes.map(pane => pane.channelId)).toEqual([C, C, D, D]);
    expect(removeClosedViewer(reversedA, { channelId: A, snapshot: afterA })).toBe(reversedA);
  });
  it('ignores stale epochs and keeps another pane recording and pending controls untouched', () => {
    const current = multiple(); current.audioOwner = B;
    current.panes[2] = { ...current.panes[2]!, recordingId: 'recording-survivor', receiverBacked: true };
    current.pendingControl = { paneId: 'video-1', id: 'pending', action: 'screenshot', channelId: B, expiresAt: 100 };
    expect(removeClosedViewer(current, { channelId: A, snapshot: { ...current, epoch: 6 } })).toBe(current);
    const next = removeClosedViewer(current, { channelId: A, snapshot: current });
    expect(next.audioOwner).toBe(B); expect(next.pendingControl).toBe(current.pendingControl);
    expect(next.panes[0]).toBe(current.panes[2]);
  });
  it('clears only a removed pane control and becomes inactive on the last viewer', () => {
    const current = active(); current.audioOwner = A;
    current.pendingUiAction = { paneId: current.panes[0]!.paneId, id: 'exit', action: 'exit_focus' };
    const next = removeClosedViewer(current, { channelId: A, snapshot: current });
    expect(next).toMatchObject({ active: false, panes: [], audioOwner: null, pendingUiAction: null });
  });
});
describe("마도 배치", () => {
  it("names the bottom handle for watching or editing the current broadcast", async () => {
    const api = fake(); api.snapshot.mockResolvedValue(success(active()));
    await act(async () => root.render(<MadoWorkspace runtime="tauri" privacy={false} unifiedLive onLeave={vi.fn()} api={api} />));
    expect(container.querySelector('.live-bottom-handle')?.textContent).toBe("");
    expect(container.querySelector('.live-bottom-handle')).toHaveAttribute("aria-label", "방송 조작 열기");
    await act(async () => root.render(null));
    api.snapshot.mockResolvedValue(success(emptyMultiview()));
    await act(async () => root.render(<MadoWorkspace runtime="tauri" privacy={false} unifiedLive onLeave={vi.fn()} api={api} />));
    expect(container.querySelector('.live-bottom-handle')?.textContent).toBe("");
    expect(container.querySelector('.live-bottom-handle')).toHaveAccessibleName("방송 보기 열기");
  });
  it("starts with a short directional guide and opens the drawer only from the bottom handle", async () => {
    const api = fake();
    await act(async () => root.render(<MadoWorkspace runtime="tauri" privacy={false} unifiedLive onLeave={vi.fn()} api={api} />));
    expect(container.querySelector('.mado-live-empty')).toHaveTextContent("아래 손잡이를 누르거나 끌어올려 방송을 선택하세요.");
    expect(container.querySelector('.live-empty-direction')).toHaveAttribute('aria-hidden', 'true');
    expect(container.querySelector('.mado-connection-panel')).toBeNull();
    const handle = button('방송 보기 열기');
    expect(handle).toBeVisible();
    await act(async () => { handle.focus(); handle.click(); });
    expect(handle).toBeVisible(); expect(handle).toHaveAccessibleName('방송 보기 닫기');
    expect(handle).toHaveAttribute('aria-expanded', 'true');
    expect(container.querySelector('.mado-live-empty')).toBeNull();
    expect(container.querySelector('.mado-connection-panel')).not.toBeNull();
    await act(async () => button('연결 설정 닫기').click());
    expect(handle).toBeVisible(); expect(document.activeElement).toBe(handle);
    expect(container.querySelector('.mado-live-empty')).not.toBeNull();
    expect(api.configure).not.toHaveBeenCalled(); expect(api.close).not.toHaveBeenCalled();
  });
  it("keeps the neutral empty-state guide readable in privacy mode without exposing channels", async () => {
    const api = fake();
    await act(async () => root.render(<MadoWorkspace runtime="tauri" privacy unifiedLive onLeave={vi.fn()} api={api} />));
    expect(container.querySelector('.mado-live-empty')).toHaveTextContent('프라이버시 모드를 끈 뒤');
    expect(button('방송 보기 열기')).toBeVisible();
    expect(container.querySelector('.mado-connection-panel,.mado-native-slot')).toBeNull();
    expect(api.configure).not.toHaveBeenCalled();
  });
  it("requires a real upward pull, not unpressed movement or a cancelled gesture", async () => {
    const api = fake();
    await act(async () => root.render(<MadoWorkspace runtime="tauri" privacy={false} unifiedLive onLeave={vi.fn()} api={api} />));
    const handle = button('방송 보기 열기');
    const pointer = (type: string, y: number) => {
      const event = new MouseEvent(type, { clientY: y, button: 0, bubbles: true });
      Object.defineProperty(event, 'pointerId', { value: 1 }); handle.dispatchEvent(event);
    };
    await act(async () => { pointer('pointermove', 30); pointer('pointerdown', 100); pointer('pointermove', 90); });
    expect(container.querySelector('.mado-connection-panel')).toBeNull();
    await act(async () => { pointer('pointercancel', 90); pointer('pointermove', 20); });
    expect(container.querySelector('.mado-connection-panel')).toBeNull();
    await act(async () => { pointer('pointerdown', 100); pointer('pointermove', 75); });
    expect(container.querySelector('.mado-connection-panel')).not.toBeNull();
    await act(async () => { pointer('pointerup', 75); handle.dispatchEvent(new MouseEvent('click', { bubbles: true, detail: 1 })); });
    expect(handle).toHaveAttribute('aria-expanded', 'true');
    await act(async () => { pointer('pointerdown', 100); pointer('pointermove', 125); pointer('pointerup', 125); handle.dispatchEvent(new MouseEvent('click', { bubbles: true, detail: 1 })); });
    expect(handle).toHaveAttribute('aria-expanded', 'false');
    await act(async () => handle.click()); expect(handle).toHaveAttribute('aria-expanded', 'true');
    await act(async () => handle.click()); expect(handle).toHaveAttribute('aria-expanded', 'false');
    expect(api.configure).not.toHaveBeenCalled();
  });
  it("routes the native chat and fullscreen buttons to Atsumi without opening another chat", async () => {
    const api = fake(); api.snapshot.mockResolvedValue(success(active()));
    nativeWindow.setFullscreen.mockClear();
    await act(async () => root.render(<MadoWorkspace runtime="tauri" privacy={false} unifiedLive onLeave={vi.fn()} api={api} />));
    const send = (action: string, channelId = A) => act(async () => { for (const handler of presentationEvents) handler({ payload: { action, channelId } }); });
    await send('chat_toggle', B);
    expect(container.querySelector('.mado-content-surfaces')).not.toHaveClass('chat-hidden');
    await send('chat_toggle');
    expect(container.querySelector('.mado-content-surfaces')).toHaveClass('chat-hidden');
    await send('chat_toggle');
    expect(container.querySelector('.mado-content-surfaces')).not.toHaveClass('chat-hidden');
    await send('fullscreen_toggle'); expect(windowFullscreen.acquire).toHaveBeenCalledExactlyOnceWith(expect.any(String), 7);
    await send('fullscreen_exit'); expect(windowFullscreen.release).toHaveBeenCalledExactlyOnceWith(expect.objectContaining({ documentEpoch: 7, revision: 1 }));
    expect(nativeWindow.setFullscreen).not.toHaveBeenCalled();
    expect(api.configure).not.toHaveBeenCalled(); expect(api.close).not.toHaveBeenCalled();
  });
  it("does not exit another provider after this fullscreen grant was superseded", async () => {
    const api = fake(); api.snapshot.mockResolvedValue(success(active()));
    await act(async () => root.render(<MadoWorkspace runtime="tauri" privacy={false} unifiedLive onLeave={vi.fn()} api={api} />));
    const send = (action: string) => act(async () => { for (const handler of presentationEvents) handler({ payload: { action, channelId: A } }); });
    await send('fullscreen_toggle');
    const acquired = await windowFullscreen.acquire.mock.results[0]!.value as WindowFullscreenOutcome;
    await act(async () => { for (const handler of fullscreenLostEvents) handler({ payload: acquired.grant! }); });
    await send('fullscreen_exit');
    expect(windowFullscreen.release).not.toHaveBeenCalled();
    await send('fullscreen_toggle'); expect(windowFullscreen.acquire).toHaveBeenCalledTimes(2);
    expect(api.close).not.toHaveBeenCalled(); expect(api.configure).not.toHaveBeenCalled();
  });
  it("releases a delayed grant after its workspace closes without adopting it", async () => {
    const api = fake(); api.snapshot.mockResolvedValue(success(active()));
    const pendingGrant = deferred<WindowFullscreenOutcome>(); windowFullscreen.acquire.mockReturnValueOnce(pendingGrant.promise);
    await act(async () => root.render(<MadoWorkspace runtime="tauri" privacy={false} unifiedLive onLeave={vi.fn()} api={api} />));
    await act(async () => { for (const handler of presentationEvents) handler({ payload: { action: 'fullscreen_toggle', channelId: A } }); });
    const [owner, documentEpoch] = windowFullscreen.acquire.mock.calls[0]!;
    await act(async () => root.render(null));
    const grant = { owner, documentEpoch, revision: 4, windowKey: 'main-test' };
    await act(async () => pendingGrant.resolve({ grant, error: null }));
    expect(windowFullscreen.release).toHaveBeenCalledExactlyOnceWith(grant);
  });
  it("does not install a grant revoked before the acquire response arrives", async () => {
    const api = fake(); api.snapshot.mockResolvedValue(success(active()));
    const pendingGrant = deferred<WindowFullscreenOutcome>(); windowFullscreen.acquire.mockReturnValueOnce(pendingGrant.promise);
    await act(async () => root.render(<MadoWorkspace runtime="tauri" privacy={false} unifiedLive onLeave={vi.fn()} api={api} />));
    const send = (action: string) => act(async () => { for (const handler of presentationEvents) handler({ payload: { action, channelId: A } }); });
    await send('fullscreen_toggle');
    const [owner, documentEpoch] = windowFullscreen.acquire.mock.calls[0]!;
    const grant = { owner, documentEpoch, revision: 5, windowKey: 'main-test' };
    await act(async () => { for (const handler of fullscreenLostEvents) handler({ payload: grant }); pendingGrant.resolve({ grant, error: null }); });
    expect(windowFullscreen.release).toHaveBeenCalledExactlyOnceWith(grant);
    await send('fullscreen_exit'); expect(windowFullscreen.release).toHaveBeenCalledTimes(1);
  });
  it("keeps audio on minimize, mutes for tray hide and restores the same viewport", async () => {
    const api = fake();
    let hidden = false;
    vi.spyOn(document, 'hidden', 'get').mockImplementation(() => hidden);
    vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockReturnValue(new DOMRect(10, 10, 800, 500));
    await act(async () => root.render(<NativeStreamingPane api={api} pane={active().panes[0]!} epoch={7} privacy={false} onError={vi.fn()} />));
    const initial = api.setViewport.mock.calls.at(-1)![1];
    expect(initial.visible).toBe(true); api.setViewport.mockClear();
    nativeWindow.isMinimized.mockResolvedValue(true); hidden = true;
    await act(async () => document.dispatchEvent(new Event('visibilitychange')));
    expect(api.setViewport).not.toHaveBeenCalled();
    nativeWindow.isVisible.mockResolvedValue(false);
    await act(async () => document.dispatchEvent(new Event('visibilitychange')));
    expect(api.setViewport.mock.calls.at(-1)![1]).toMatchObject({ visible: false, suspendAudio: true });
    hidden = false; nativeWindow.isVisible.mockResolvedValue(true); nativeWindow.isMinimized.mockResolvedValue(false);
    await act(async () => document.dispatchEvent(new Event('visibilitychange')));
    expect(api.setViewport.mock.calls.at(-1)![1]).toEqual(initial);
  });
  it("toggles the live settings disclosure without reconnecting or stopping the current broadcast", async () => {
    const api = fake(), changed = vi.fn(); api.snapshot.mockResolvedValue(success(active()));
    await act(async () => root.render(<WorkspaceFixture runtime="tauri" privacy={false} unifiedLive onLeave={() => {}} onConnectionOpenChange={changed} api={api} />));
    expect(container.querySelector('#chzzk-connection-panel')).toBeNull();
    await act(async () => button('방송 보기').click());
    expect(container.querySelector('#chzzk-connection-panel')).not.toBeNull();
    expect(changed).toHaveBeenLastCalledWith(true);
    await act(async () => button('방송 보기').click());
    expect(container.querySelector('#chzzk-connection-panel')).toBeNull();
    expect(changed).toHaveBeenLastCalledWith(false);
    expect(api.configure).not.toHaveBeenCalled(); expect(api.close).not.toHaveBeenCalled();
  });
  it("has no live header and shares one action row for sliding mode, start, change and stop", async () => {
    const api = fake();
    await act(async () => root.render(<WorkspaceFixture runtime="tauri" privacy={false} unifiedLive onLeave={() => {}} api={api} />));
    expect(container.querySelector('.mado-workspace header,.mado-workspace .edge-top')).toBeNull();
    expect(button('시청 종료')).toBeUndefined(); expect(button('위아래 배치')).toBeUndefined();
    await act(async () => button('방송 보기 열기').click());
    expect(button('모두 보기').closest('.connection-actions')).toBe(button('시청 시작').closest('.connection-actions'));
    await enter(0, A); await act(async () => button('시청 시작').click());
    await act(async () => button('방송 보기').click());
    expect(button('시청 시작')).toBeUndefined();
    expect(button('변경').closest('.connection-actions')).toBe(button('종료').closest('.connection-actions'));
    const modes = container.querySelector('.mado-layout-options')!;
    await act(async () => button('선택 보기').click());
    expect(modes).toHaveAttribute('data-mode', 'chats');
    expect(button('선택 보기')).toHaveAttribute('aria-pressed', 'true');
    expect(api.configure).toHaveBeenCalledTimes(1);
    await act(async () => button('종료').click());
    expect(api.close).toHaveBeenCalledExactlyOnceWith(7);
    expect(button('시청 시작')).toBeEnabled();
    expect(button('변경')).toBeUndefined(); expect(button('종료')).toBeUndefined();
    expect(api.requestControl).not.toHaveBeenCalled(); expect(api.confirmControl).not.toHaveBeenCalled();
  });
  it("retries temporary 2-to-4 layout contention with the same draft while retaining old panes", async () => {
    const api = fake(); const old = { ...multiple(), panes: multiple().panes.filter(pane => [A, B].includes(pane.channelId)) };
    api.snapshot.mockResolvedValue(success(old));
    api.configure.mockResolvedValueOnce({ ok: false, error: { code: "MULTIVIEW_BUSY", message: "잠시 사용 중", retryable: true } }).mockResolvedValueOnce(success(multiple()));
    await act(async () => root.render(<WorkspaceFixture runtime="tauri" privacy={false} unifiedLive onLeave={() => {}} api={api} />));
    const nodes = [...container.querySelectorAll('.mado-native-slot')];
    await act(async () => button('방송 보기').click());
    const add = () => container.querySelector<HTMLButtonElement>('[aria-label="방송 추가"]')!;
    await act(async () => add().click()); await enter(2, C);
    await act(async () => add().click()); await enter(3, D);
    await act(async () => button('변경').click());
    expect([...container.querySelectorAll('.mado-native-slot')]).toEqual(nodes);
    expect(button('변경')).toBeDisabled();
    expect(button('변경')).toHaveAttribute('aria-busy', 'true');
    await act(async () => vi.advanceTimersByTimeAsync(300));
    expect(api.configure).toHaveBeenCalledTimes(2);
    expect(api.configure.mock.calls[0]).toEqual(api.configure.mock.calls[1]);
    expect(container.querySelectorAll('.mado-native-slot')).toHaveLength(8);
    expect(container.querySelector('[role=alert]')).toBeNull();
    expect(api.close).not.toHaveBeenCalled();
  });
  it("bounds busy retries and keeps settings usable after the last rejection", async () => {
    const api = fake();
    api.configure.mockResolvedValue({ ok: false, error: { code: 'MULTIVIEW_BUSY', message: '잠시 후 다시 시도', retryable: true } });
    await act(async () => root.render(<WorkspaceFixture runtime="tauri" privacy={false} onLeave={() => {}} api={api} />));
    await enter(0, A); await act(async () => button('시청 시작').click());
    await act(async () => vi.advanceTimersByTimeAsync(5000));
    expect(api.configure).toHaveBeenCalledTimes(4);
    expect(button('시청 시작')).toBeEnabled();
    expect(container.querySelector('[role=alert]')).toHaveTextContent('잠시 후 다시 시도');
  });
  it("resizes video and chat splits independently without reconfiguring streams", async () => {
    localStorage.setItem('atsumi.live.arrangement.v1', 'grouped');
    const api = fake(); api.snapshot.mockResolvedValue(success(multiple()));
    await act(async () => root.render(<WorkspaceFixture runtime="tauri" privacy={false} onLeave={() => {}} api={api} />));
    const panes = [...container.querySelectorAll('.mado-native-slot')];
    for (const name of ['방송 좌우 크기', '방송 상하 크기', '채팅 좌우 크기', '채팅 상하 크기']) {
      const separator = container.querySelector(`[aria-label="${name}"]`)!;
      await act(async () => separator.dispatchEvent(new KeyboardEvent('keydown', { key: name.includes('좌우') ? 'ArrowRight' : 'ArrowDown', bubbles: true })));
      expect(separator).toHaveAttribute('aria-valuenow', '52');
    }
    expect(container.querySelector('[aria-label="영상과 채팅 크기"]')).toHaveAttribute('aria-valuenow', '60');
    await act(async () => vi.advanceTimersByTimeAsync(220));
    expect(JSON.parse(localStorage.getItem('atsumi.live.panes.video.v1')!)).toMatchObject({ columns: 52, rows: 52 });
    expect([...container.querySelectorAll('.mado-native-slot')]).toEqual(panes);
    expect(api.configure).not.toHaveBeenCalled(); expect(api.close).not.toHaveBeenCalled(); expect(api.setAudio).not.toHaveBeenCalled();
  });
  it("pairs each broadcast with its own chat and resizes each pair without reconnecting", async () => {
    const api = fake(); api.snapshot.mockResolvedValue(success(multiple()));
    await act(async () => root.render(<WorkspaceFixture runtime="tauri" privacy={false} unifiedLive onLeave={() => {}} api={api} />));
    const groups = [...container.querySelectorAll('.live-channel-pair')];
    expect(groups).toHaveLength(4);
    for (const [index, group] of groups.entries()) {
      expect(group).toHaveAttribute('aria-label', `${index + 1}. 도서관 ${index + 1}`);
      expect([...group.querySelectorAll('.mado-native-slot')].map(node => node.getAttribute('aria-label'))).toEqual(['방송 화면', '채팅 화면']);
      expect(group.querySelector('.mado-chat-cell')?.id).toBe(`mado-chat-panel-${[A, B, C, D][index]}`);
    }
    const nodes = [...container.querySelectorAll('.mado-native-slot')];
    const first = container.querySelector('[aria-label="1번 방송과 채팅 좌우 크기"]')!;
    await act(async () => first.dispatchEvent(new KeyboardEvent('keydown', {key:'ArrowRight',bubbles:true})));
    expect(first).toHaveAttribute('aria-valuenow','52');
    expect(container.querySelector('[aria-label="2번 방송과 채팅 좌우 크기"]')).toHaveAttribute('aria-valuenow','50');
    expect([...container.querySelectorAll('.mado-native-slot')]).toEqual(nodes);
    expect(api.configure).not.toHaveBeenCalled(); expect(api.close).not.toHaveBeenCalled(); expect(api.setAudio).not.toHaveBeenCalled();
  });
  it("switches a compact chat selection without reconfiguring or unmounting connected receivers", async () => {
    const api = fake(); api.snapshot.mockResolvedValue(success(multiple()));
    await act(async () => root.render(<WorkspaceFixture runtime="tauri" privacy={false} unifiedLive onLeave={() => {}} api={api} />));
    const panes = [...container.querySelectorAll('.mado-native-slot')];
    const tabs = [...container.querySelectorAll<HTMLButtonElement>('[role="tab"]')];
    expect(tabs).toHaveLength(4); expect(tabs[0]).toHaveAttribute("aria-selected", "true");
    await act(async () => tabs[2]!.click());
    expect(tabs[2]).toHaveAttribute("aria-selected", "true");
    expect(container.querySelector('[data-chat-selected=true]')).toHaveAttribute("id", `mado-chat-panel-${C}`);
    expect([...container.querySelectorAll('.mado-native-slot')]).toEqual(panes);
    await act(async () => tabs[2]!.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true })));
    expect(tabs[3]).toHaveAttribute("aria-selected", "true"); expect(document.activeElement).toBe(tabs[3]);
    expect(api.configure).not.toHaveBeenCalled(); expect(api.close).not.toHaveBeenCalled(); expect(api.setAudio).not.toHaveBeenCalled();
    expect(container.querySelector('[role="separator"]')).not.toBeNull();
  });
  it("starts unified live with one editable row and adds/removes at most four broadcasts", async () => {
    const api = fake();
    await act(async () => root.render(<WorkspaceFixture runtime="tauri" privacy={false} unifiedLive onLeave={() => {}} api={api} />));
    await act(async () => button('방송 보기 열기').click());
    expect(container.querySelectorAll('.connection-channel-draft input')).toHaveLength(1);
    expect(button("일반모드")).toBeUndefined(); expect(button("마도모드")).toBeUndefined();
    const add = () => container.querySelector<HTMLButtonElement>('[aria-label="방송 추가"]')!;
    for (let count = 2; count <= 4; count++) { await act(async () => add().click()); expect(container.querySelectorAll('.connection-channel-draft input')).toHaveLength(count); }
    expect(add()).toBeDisabled();
    await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="방송 2 입력칸 제거"]')!.click());
    expect(container.querySelectorAll('.connection-channel-draft input')).toHaveLength(3); expect(add()).toBeEnabled();
    expect(api.configure).not.toHaveBeenCalled(); expect(api.close).not.toHaveBeenCalled();
  });
  it("keeps replacement addresses editable after closing a receiver-backed four-channel view", async () => {
    const state = multiple();
    for (const pane of state.panes) { pane.receiverBacked = true; if (pane.channelId === A && pane.kind === "video") pane.recordingStatus = "recording"; }
    const api = fake(); api.snapshot.mockResolvedValue(success(state));
    // Stale display panes can remain in a closing snapshot but are no longer an
    // active layout and must not lock the form against the next connection.
    api.close.mockResolvedValue(success({ ...state, active: false, epoch: 8 }));
    await act(async () => root.render(<WorkspaceFixture runtime="tauri" privacy={false} unifiedLive onLeave={() => {}} api={api} />));
    await act(async () => button("방송 보기").click());
    await act(async () => button("종료").click());
    const replacement = "e".repeat(32);
    await enter(1, replacement);
    expect(container.querySelector<HTMLInputElement>('#mado-channel-1')).toHaveValue(replacement);
    await act(async () => vi.advanceTimersByTimeAsync(2000));
    // An explicitly open editor survives late snapshots; closing and reopening
    // it must still retain the changed draft.
    expect(container.querySelector<HTMLInputElement>('#mado-channel-1')).toHaveValue(replacement);
    await act(async () => button('연결 설정 닫기').click());
    await act(async () => button("방송 보기").click());
    expect(container.querySelector<HTMLInputElement>('#mado-channel-1')).toHaveValue(replacement);
    await act(async () => button("변경").click());
    expect(api.configure).toHaveBeenLastCalledWith([A, replacement, C, D].map(channelId => ({ channelId, video: true, chat: true })));
    expect(api.requestControl).not.toHaveBeenCalled(); expect(api.confirmControl).not.toHaveBeenCalled();
  });
  it("keeps integrated auto-record controls available during the tutorial without stopping playback", async () => {
    const api = fake(); api.snapshot.mockResolvedValue(success(active()));
    await act(async () => root.render(<WorkspaceFixture runtime="tauri" privacy={false} unifiedLive tutorialActive onLeave={() => {}} api={api} />));
    await act(async () => button("방송 보기").click());
    expect(container.querySelector('[role="dialog"]')).not.toBeNull();
    try {
      await act(async () => {
        document.documentElement.dataset.tutorialStep = "chzzk-auto";
        window.dispatchEvent(new CustomEvent(tutorialStepEvent));
      });
      expect(container.querySelector('[data-tour="chzzk-auto"]')).not.toBeNull();
      expect(api.close).not.toHaveBeenCalled(); expect(api.configure).not.toHaveBeenCalled();
    } finally { delete document.documentElement.dataset.tutorialStep; }
  });
  it("shows only explicitly opened tutorial settings in privacy mode, without starting playback", async () => {
    const api = fake(); api.snapshot.mockResolvedValue(success(active()));
    await act(async () => root.render(<WorkspaceFixture runtime="tauri" privacy tutorialActive unifiedLive onLeave={() => {}} api={api} />));
    expect(container.querySelector('[data-tour="chzzk-connection"]')).toBeNull();
    await act(async () => button("방송 보기").click());
    expect(container.querySelector('[data-tour="chzzk-connection"]')).not.toBeNull();
    expect(container.querySelectorAll(".mado-native-slot")[0]).toHaveTextContent("프라이버시 모드");
    expect(api.configure).not.toHaveBeenCalled(); expect(api.setAudio).not.toHaveBeenCalled(); expect(api.close).not.toHaveBeenCalled();
    await act(async () => root.render(<WorkspaceFixture runtime="tauri" privacy tutorialActive={false} unifiedLive onLeave={() => {}} api={api} />));
    expect(container.querySelector('[data-tour="chzzk-connection"]')).toBeNull();
  });
  it.each([false, true])("starts with a guide for unified live, retaining the legacy inline setup without stealing focus (unified: %s)", async unifiedLive => {
    const api = fake(), background = document.createElement("button"); document.body.append(background); background.focus();
    try {
      const render = () => root.render(<WorkspaceFixture runtime="tauri" privacy={false} unifiedLive={unifiedLive} onLeave={() => {}} api={api} />);
      await act(async () => render());
      expect(container.querySelector(unifiedLive ? ".mado-live-empty" : ".mado-connection-inline .connection-setup")).not.toBeNull();
      expect(container.querySelector('[role="dialog"]')).toBeNull();
      expect(container.querySelector('[aria-label="연결 설정 닫기"]')).toBeNull();
      expect(document.activeElement).toBe(background);
      expect(api.configure).not.toHaveBeenCalled(); expect(api.close).not.toHaveBeenCalled();
      await act(async () => root.render(null));
      await act(async () => render());
      expect(container.querySelector('[role="dialog"]')).toBeNull();
      expect(document.activeElement).toBe(background);
    } finally { background.remove(); }
  });
  it("opens a nonmodal channel panel, lets Tab leave it and respects background focus when closing", async () => {
    const api = fake(); api.snapshot.mockResolvedValue(success(active()));
    const background = document.createElement("button"); document.body.append(background);
    try {
      await act(async () => root.render(<WorkspaceFixture runtime="tauri" privacy={false} unifiedLive onLeave={() => {}} api={api} />));
      const opener = button("방송 보기"); await act(async () => opener.focus());
      await act(async () => opener.click());
      const dialog = container.querySelector('.mado-connection-panel[role="dialog"]')!;
      expect(dialog).toHaveAttribute("aria-modal", "false");
      expect(dialog).toHaveAttribute("data-native-overlay", "true");
      expect(dialog).toHaveAttribute("data-native-preserve-video", "true");
      expect(dialog).toHaveAttribute("data-native-interactive-background", "true");
      expect(document.activeElement).toBe(button("연결 설정 닫기"));
      const last = button("로그아웃"); last.focus();
      const tab = new KeyboardEvent("keydown", { key: "Tab", bubbles: true, cancelable: true });
      await act(async () => last.dispatchEvent(tab));
      expect(tab.defaultPrevented).toBe(false);
      const first = button("연결 설정 닫기"); first.focus();
      const previous = new KeyboardEvent("keydown", { key: "Tab", shiftKey: true, bubbles: true, cancelable: true });
      await act(async () => first.dispatchEvent(previous));
      expect(previous.defaultPrevented).toBe(false);
      background.focus();
      await act(async () => button("연결 설정 닫기").click());
      expect(document.activeElement).toBe(background);
      expect(container.querySelector('.mado-connection-panel')).toBeNull();
      await act(async () => { opener.focus(); opener.click(); });
      await act(async () => button("연결 설정 닫기").dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })));
      expect(document.activeElement).toBe(opener);
      expect(container.querySelector('.mado-connection-panel')).toBeNull();
      expect(api.configure).not.toHaveBeenCalled(); expect(api.close).not.toHaveBeenCalled();
    } finally { background.remove(); }
  });
  it("keeps an explicitly opened panel open when the initial active snapshot arrives", async () => {
    const api = fake(), initial = deferred<Awaited<ReturnType<MultiviewApi["snapshot"]>>>();
    api.snapshot.mockReturnValue(initial.promise);
    await act(async () => root.render(<WorkspaceFixture runtime="tauri" privacy={false} unifiedLive onLeave={() => {}} api={api} />));
    await act(async () => button("방송 보기").click());
    await act(async () => initial.resolve(success(active())));
    expect(container.querySelector('[role="dialog"]')).not.toBeNull();
    expect(container.querySelectorAll(".mado-native-slot")).toHaveLength(2);
  });
  it("routes a single pasted URL through the unified resolver, never the legacy player", async () => {
    const api = fake();
    const officialApi = { ...createOfficialBrowserApi("tauri"), open: vi.fn() };
    await act(async () => root.render(<WorkspaceFixture runtime="tauri" privacy={false} unifiedLive onLeave={() => {}} api={api} officialApi={officialApi} />));
    await act(async () => button('방송 보기 열기').click());
    const input = container.querySelector<HTMLInputElement>("#mado-channel-0")!;
    await act(async () => { Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, `https://chzzk.naver.com/live/${A}`); input.dispatchEvent(new Event("input", { bubbles: true })); });
    await act(async () => button("시청 시작").click());
    expect(api.configure).toHaveBeenCalledExactlyOnceWith([{ channelId: A, video: true, chat: true }]);
    expect(officialApi.open).not.toHaveBeenCalled();
  });
  it("mixes two recording receivers with an ordinary stream and closes only presentation", async () => {
    const state = multiple(); state.panes = state.panes.filter(pane => pane.channelId !== D);
    for (const pane of state.panes) if (pane.kind === "video") { pane.receiverBacked = true; if (pane.channelId !== C) { pane.recordingStatus = "recording"; pane.recordingId = `record-${pane.channelId}`; } }
    const api = fake(); api.snapshot.mockResolvedValue(success(state)); api.configure.mockResolvedValue(success(state));
    await act(async () => root.render(<WorkspaceFixture runtime="tauri" privacy={false} unifiedLive onLeave={() => {}} api={api} />));
    expect(container.querySelectorAll('.mado-native-slot.is-video')).toHaveLength(3);
    await act(async () => button("방송 보기").click());
    expect(button("변경")).toBeEnabled(); expect(button("마도모드")).toBeUndefined(); expect(button("로그인")).toBeEnabled(); expect(button("로그아웃")).toBeDisabled();
    await act(async () => button("변경").click());
    expect(api.configure).toHaveBeenCalledExactlyOnceWith([A, B, C].map(channelId => ({ channelId, video: true, chat: true })));
    await act(async () => button("방송 보기").click());
    await act(async () => button("종료").click());
    expect(api.close).toHaveBeenCalledExactlyOnceWith(7); expect(api.requestControl).not.toHaveBeenCalled(); expect(api.confirmControl).not.toHaveBeenCalled();
    expect(container.querySelector('.mado-connection-panel[role="dialog"]')).not.toBeNull();
    expect(container.querySelector(".mado-connection-panel .connection-setup")).not.toBeNull();
  });
  it("needs no grid connection with a live receiver or the old root player", async () => {
    const api = fake(); api.snapshot.mockResolvedValue(success(active()));
    const officialApi = { ...createOfficialBrowserApi("tauri"), snapshot: vi.fn().mockResolvedValue(success(emptyOfficialBrowserSnapshot("tauri"))), connectExtension: vi.fn().mockResolvedValue(success(emptyOfficialBrowserSnapshot("tauri"))) };
    await act(async () => root.render(<WorkspaceFixture runtime="tauri" privacy={false} unifiedLive onLeave={() => {}} api={api} officialApi={officialApi} />));
    await act(async () => button("방송 보기").click()); expect(button("그리드 연결")).toBeUndefined();
    await act(async () => container.querySelector<HTMLButtonElement>('.connection-help')!.click());
    expect(document.querySelector('.connection-help-content')).toHaveTextContent("그리드 없이 직접 재생"); expect(officialApi.connectExtension).not.toHaveBeenCalled(); expect(api.close).not.toHaveBeenCalled();
  });
  it("automatically rechecks the account and displays failures without changing the Mado panes", async () => {
    const api = fake(); api.snapshot.mockResolvedValue(success(multiple()));
    const account = { ...createOfficialBrowserApi("tauri"), snapshot: vi.fn().mockResolvedValue(success({ ...emptyOfficialBrowserSnapshot("tauri"), authStatus: "unknown" })) };
    await act(async () => root.render(<WorkspaceFixture runtime="tauri" privacy={false} onLeave={() => {}} api={api} officialApi={account} />));
    await act(async () => button("연결 설정").click());
    const panes = [...container.querySelectorAll('.mado-native-slot')];
    const fresh = deferred<ReturnType<typeof success>>();
    account.snapshot.mockReturnValueOnce(fresh.promise);
    await act(async () => vi.advanceTimersByTimeAsync(2000));
    expect(account.snapshot).toHaveBeenLastCalledWith();
    expect(button("상태 확인")).toBeUndefined();
    await act(async () => fresh.resolve(success({ ...emptyOfficialBrowserSnapshot("tauri"), authStatus: "unknown", authError: "로그인 상태 조회 실패" })));
    expect(container).toHaveTextContent("로그인 상태 조회 실패");
    expect(button("상태 확인")).toBeUndefined();
    expect([...container.querySelectorAll('.mado-native-slot')]).toEqual(panes);
    expect(api.configure).not.toHaveBeenCalled(); expect(api.close).not.toHaveBeenCalled();
  });

  it("opens login and logs out directly during Mado viewing without changing its layout", async () => {
    const api = fake(); api.snapshot.mockResolvedValue(success(multiple()));
    const account = { ...createOfficialBrowserApi("tauri"),
      snapshot: vi.fn().mockResolvedValue(success({ ...emptyOfficialBrowserSnapshot("tauri"), authStatus: "signed_out" })),
      login: vi.fn().mockResolvedValue(success({ ...emptyOfficialBrowserSnapshot("tauri"), authStatus: "signed_in" })),
      logout: vi.fn().mockResolvedValue(success({ ...emptyOfficialBrowserSnapshot("tauri"), authStatus: "signed_out" })),
    };
    await act(async () => root.render(<WorkspaceFixture runtime="tauri" privacy={false} onLeave={() => {}} api={api} officialApi={account} />));
    await act(async () => button("연결 설정").click());
    const panes = [...container.querySelectorAll('.mado-native-slot')];
    expect(panes).toHaveLength(8);
    expect(button("로그인")).toBeEnabled();
    await act(async () => button("로그인").click());
    expect(account.login).toHaveBeenCalledOnce();
    expect(button("로그아웃")).toBeEnabled();
    await act(async () => button("로그아웃").click());
    expect(account.logout).toHaveBeenCalledOnce();
    expect(container).toHaveTextContent("로그아웃됨");
    expect(container.querySelector('[role="alertdialog"]')).toBeNull();
    expect([...container.querySelectorAll('.mado-native-slot')]).toEqual(panes);
    expect(api.close).not.toHaveBeenCalled(); expect(api.configure).not.toHaveBeenCalled();
  });

  it.each(["recording", "account"])("blocks account actions while %s is busy, not just because Mado is active", async (reason) => {
    const snapshot = multiple(); if (reason === "recording") snapshot.panes[0]!.recordingStatus = "recording";
    const api = fake(); api.snapshot.mockResolvedValue(success(snapshot));
    const account = { ...createOfficialBrowserApi("tauri"), snapshot: vi.fn().mockResolvedValue(success({ ...emptyOfficialBrowserSnapshot("tauri"), authStatus: "unknown", accountBusy: reason === "account" })), login: vi.fn(), logout: vi.fn() };
    await act(async () => root.render(<WorkspaceFixture runtime="tauri" privacy={false} onLeave={() => {}} api={api} officialApi={account} />));
    await act(async () => button("연결 설정").click());
    if (reason === "recording") expect(button("로그인")).toBeEnabled(); else expect(button("로그인")).toBeDisabled();
    expect(button("로그아웃")).toBeDisabled();
    await act(async () => { button("로그인").click(); button("로그아웃").click(); });
    if (reason === "recording") expect(account.login).toHaveBeenCalledOnce(); else expect(account.login).not.toHaveBeenCalled();
    expect(account.logout).not.toHaveBeenCalled();
  });

  it("numbers URL fields while retaining accessible names and the requested layout names", async () => {
    const api = fake();
    await act(async () => root.render(<WorkspaceFixture runtime="tauri" privacy={false} onLeave={() => {}} api={api} />));
    expect(button("모두 보기")).toHaveAttribute("aria-pressed", "true");
    expect(button("선택 보기")).toHaveAttribute("aria-pressed", "false");
    for (const [index, input] of [...container.querySelectorAll<HTMLInputElement>('.connection-channel-inputs input')].entries()) {
      expect(input).toHaveAccessibleName(`방송 ${index + 1} 주소 또는 ID`);
      expect(input.placeholder).toBe("방송·채널 주소 또는 채널 ID");
      expect(input.labels?.[0]).toHaveClass("connection-input-label-hidden");
      expect(input.closest('.connection-channel-row')?.querySelector('.connection-channel-number')).toHaveTextContent(String(index + 1));
    }
  });
  it("shows hover help outside the form without shifting connection controls", async () => {
    const api = fake();
    await act(async () => root.render(<WorkspaceFixture runtime="tauri" privacy={false} onLeave={() => {}} api={api} />));
    const help = container.querySelector<HTMLButtonElement>('.connection-help')!;
    expect(document.querySelector('.connection-help-popover')).toBeNull();
    const form = container.querySelector('form');
    expect(button("Chrome")).toBeUndefined();
    await act(async () => help.dispatchEvent(new MouseEvent("mouseover", { bubbles: true })));
    expect(help).toHaveAttribute("aria-expanded", "true");
    const content = document.querySelector<HTMLElement>('.connection-help-content')!;
    expect(container.contains(content)).toBe(false);
    expect(content.parentElement).toHaveAttribute('data-native-preserve-video', 'true');
    expect(container.querySelector('form')).toBe(form);
    expect(button("Chrome")).toBeUndefined();
    await act(async () => help.focus());
    expect(help).toHaveAccessibleDescription(/주소 입력 후 시청 시작/);
    expect(content.querySelectorAll('p')).toHaveLength(5);
    await act(async () => help.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })));
    expect(document.activeElement).toBe(help); expect(button("Chrome")).toBeUndefined();
    expect(document.querySelector('.connection-help-popover')).toBeNull();
    expect(container.querySelector('[aria-label="연결 설정"][role="dialog"]')).toBeNull();
    expect(api.configure).not.toHaveBeenCalled();
  });
  it.each([false, true])("keeps video and chat panes free of duplicate outside headers and audio controls (one video: %s)", async (oneVideo) => {
    const api = fake(); api.snapshot.mockResolvedValue(success(multiple(oneVideo)));
    await act(async () => root.render(<WorkspaceFixture runtime="tauri" privacy={false} onLeave={() => {}} api={api} />));
    expect(container.querySelector('.mado-pair > .mado-pane-label, .mado-lead-inner > .mado-pane-label')).toBeNull();
    expect(container.querySelectorAll('.mado-pane-label, .mado-audio, .mado-pane-name')).toHaveLength(0);
    expect(container.querySelectorAll('.mado-chat-cell')).toHaveLength(4);
    expect([...container.querySelectorAll('.mado-chat-cell')].every(cell => cell.children.length === 1 && cell.firstElementChild?.classList.contains('mado-native-slot'))).toBe(true);
    expect(container.querySelectorAll('.mado-native-slot.is-video')).toHaveLength(oneVideo ? 1 : 4);
  });
  it("keeps recording problems out of the live viewing surface", async () => {
    const api = fake(); let state = multiple();
    state.panes[0] = { ...state.panes[0]!, recordingStatus: "stopped", recordingId: null, chatStatus: "storage_failed", chatCount: 42 };
    api.snapshot.mockImplementation(async () => success(state));
    await act(async () => root.render(<WorkspaceFixture runtime="tauri" privacy={false} onLeave={() => {}} api={api} />));
    const slots = [...container.querySelectorAll(".mado-native-slot")];
    expect(container.querySelector(".mado-chat-warning,.mado-notices")).toBeNull();
    state = { ...state, panes: state.panes.map((pane, index) => index === 0 ? { ...pane, chatStatus: "connection_gap" } : pane) };
    await act(async () => vi.advanceTimersByTimeAsync(2000));
    expect(container.querySelector(".mado-chat-warning,.mado-notices")).toBeNull();
    expect([...container.querySelectorAll(".mado-native-slot")]).toEqual(slots);
    expect(api.close).not.toHaveBeenCalled();
  });
  it("rejects mismatched and expired per-pane intents without approval", async () => {
    const api = fake(); let state = multiple();
    state.pendingControl = { paneId: "video-0", id: "mismatch", action: "screenshot", channelId: B, expiresAt: Date.now() + 20000 };
    api.snapshot.mockImplementation(async () => success(state));
    await act(async () => root.render(<WorkspaceFixture runtime="tauri" privacy={false} onLeave={() => {}} api={api} />));
    expect(container.querySelector('[role="alertdialog"]')).toBeNull();
    state = { ...state, pendingControl: { paneId: "video-0", id: "expires", action: "screenshot", channelId: A, expiresAt: Date.now() + 2500 } };
    await act(async () => vi.advanceTimersByTimeAsync(2000)); expect(container.querySelector('[role="alertdialog"]')).not.toBeNull();
    await act(async () => vi.advanceTimersByTimeAsync(501));
    expect(container.querySelector('[role="alertdialog"]')).toBeNull(); expect(api.confirmControl).not.toHaveBeenCalled();
  });
  it("keeps recordings alive on unmount and blocks explicit configuration or leave", async () => {
    const api = fake(), leave = vi.fn(), state = multiple(); state.panes[0]!.recordingId = "test-record"; state.panes[0]!.recordingStatus = "recording";
    api.snapshot.mockResolvedValue(success(state));
    await act(async () => root.render(<WorkspaceFixture runtime="tauri" privacy={false} onLeave={leave} api={api} />));
    await act(async () => button("연결 설정").click());
    expect(button("일반모드")).toBeDisabled(); expect(button("시청 시작")).toBeDisabled();
    await act(async () => root.render(null));
    expect(api.close).not.toHaveBeenCalled(); expect(leave).not.toHaveBeenCalled();
    expect(api.setViewport.mock.calls.slice(-state.panes.length).every(([, viewport]) => !viewport.visible)).toBe(true);
  });
  it("targets the sole video for app-level S and keeps saving behind confirmation", async () => {
    const api = fake(), state = multiple(true); api.snapshot.mockResolvedValue(success(state));
    api.requestControl.mockImplementation(async (paneId, action) => success({ ...state, pendingControl: { paneId, id: "key-save", action, channelId: A, expiresAt: Date.now() + 20000 } }));
    await act(async () => root.render(<WorkspaceFixture runtime="tauri" privacy={false} onLeave={() => {}} api={api} />));
    await act(async () => window.dispatchEvent(new KeyboardEvent("keydown", { key: "s", bubbles: true })));
    expect(api.requestControl).toHaveBeenCalledExactlyOnceWith("video-0", "screenshot", 7);
    expect(button("저장 확인")).toBeEnabled(); expect(api.confirmControl).not.toHaveBeenCalled();
    expect(container.querySelector('[role="alertdialog"]')).toHaveAttribute("aria-modal", "true");
    await act(async () => container.querySelector('[role="alertdialog"]')!.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })));
    expect(api.confirmControl.mock.calls[0]?.[0]).toMatchObject({ paneId: "video-0", approve: false, rightsAcknowledged: false });
  });
  it("keeps background S available with the channel panel open while protecting text entry and real modals", async () => {
    const api = fake(), state = active(); api.snapshot.mockResolvedValue(success(state)); api.requestControl.mockResolvedValue(success(state));
    const background = document.createElement("button"), modal = document.createElement("div"); document.body.append(background);
    try {
      await act(async () => root.render(<WorkspaceFixture runtime="tauri" privacy={false} unifiedLive onLeave={() => {}} api={api} />));
      await act(async () => button("방송 보기").click());
      const input = container.querySelector<HTMLInputElement>("#mado-channel-0")!;
      input.focus(); await act(async () => input.dispatchEvent(new KeyboardEvent("keydown", { key: "s", bubbles: true })));
      expect(api.requestControl).not.toHaveBeenCalled();
      background.focus(); await act(async () => background.dispatchEvent(new KeyboardEvent("keydown", { key: "s", bubbles: true })));
      expect(api.requestControl).toHaveBeenCalledExactlyOnceWith("mado-7-a-video", "screenshot", 7);
      modal.setAttribute("role", "alertdialog"); modal.setAttribute("aria-modal", "true");
      await act(async () => document.body.append(modal));
      await act(async () => background.dispatchEvent(new KeyboardEvent("keydown", { key: "s", bubbles: true })));
      expect(api.requestControl).toHaveBeenCalledTimes(1);
    } finally { modal.remove(); background.remove(); }
  });
  it("persists bounded split preferences and supports keyboard adjustment", async () => {
    localStorage.setItem("atsumi.mado.layout.v1", '{"version":1,"direction":"horizontal","horizontal":999,"vertical":-20}');
    expect(readMadoLayout()).toEqual({ version: 1, direction: "horizontal", horizontal: 94, vertical: 10 });
    localStorage.setItem("atsumi.mado.layout.v1", "broken"); expect(readMadoLayout().horizontal).toBe(60); localStorage.clear();
    const api = fake(); api.snapshot.mockResolvedValue(success(multiple(true)));
    await act(async () => root.render(<WorkspaceFixture runtime="tauri" privacy={false} onLeave={() => {}} api={api} />));
    const divider = container.querySelector('[role="separator"]')!;
    await act(async () => divider.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true })));
    expect(divider).toHaveAttribute("aria-valuenow", "62");
    await act(async () => button("위아래 배치").click()); expect(divider).toHaveAttribute("aria-orientation", "horizontal");
    await act(async () => divider.dispatchEvent(new KeyboardEvent("keydown", { key: "Home", bubbles: true })));
    await act(async () => vi.advanceTimersByTimeAsync(250));
    expect(JSON.parse(localStorage.getItem("atsumi.mado.layout.v1")!)).toEqual({ version: 1, direction: "vertical", horizontal: 62, vertical: 10 });
  });
  it("keeps video pixels and audio during divider dragging and restores input after cancellation", async () => {
    const api = fake(); api.snapshot.mockResolvedValue(success(multiple(true)));
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue(new DOMRect(0, 0, 800, 600));
    await act(async () => root.render(<WorkspaceFixture runtime="tauri" privacy={false} onLeave={() => {}} api={api} />));
    const divider = container.querySelector('[role="separator"]')!;
    const pointer = (type: string, x: number) => { const event = new MouseEvent(type, { clientX: x, button: 0, bubbles: true }); Object.defineProperty(event, "pointerId", { value: 1 }); divider.dispatchEvent(event); };
    await act(async () => pointer("pointerdown", 480));
    expect(api.setViewport.mock.calls.slice(-5).every(([, viewport]) => viewport.visible && viewport.occluded && viewport.preserveBackground && !viewport.suspendAudio && viewport.occlusions?.length === 0)).toBe(true);
    await act(async () => { for (let i = 0; i < 20; i++) pointer("pointermove", 400 + i); });
    expect(divider).toHaveAttribute("aria-valuenow", "60");
    await act(async () => vi.advanceTimersByTimeAsync(20)); expect(divider).toHaveAttribute("aria-valuenow", "52");
    await act(async () => pointer("pointercancel", 419));
    expect(api.setViewport.mock.calls.slice(-5).every(([, viewport]) => viewport.visible && !viewport.occluded)).toBe(true);
    expect(api.close).not.toHaveBeenCalled(); expect(api.confirmControl).not.toHaveBeenCalled();
  });
  it("coalesces drag geometry, never tears down the receiver, and respects privacy", async () => {
    const api = fake(), flight = deferred<Awaited<ReturnType<MultiviewApi['setViewport']>>>();
    const bounds = vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockReturnValue(new DOMRect(0, 0, 800, 600));
    const pane = active().panes[0]!;
    await act(async () => root.render(<NativeStreamingPane api={api} pane={pane} epoch={7} privacy={false} onError={() => {}} />));
    api.setViewport.mockReturnValueOnce(flight.promise);
    await act(async () => root.render(<NativeStreamingPane api={api} pane={pane} epoch={7} privacy={false} inputBlocked onError={() => {}} />));
    const count = api.setViewport.mock.calls.length;
    for (let index = 0; index < 10; index++) {
      bounds.mockReturnValue(new DOMRect(0, 0, 700 - index, 500));
      await act(async () => { window.dispatchEvent(new Event('resize')); await vi.advanceTimersByTimeAsync(20); });
    }
    expect(api.setViewport).toHaveBeenCalledTimes(count);
    await act(async () => flight.resolve(success(undefined)));
    expect(api.setViewport.mock.calls.at(-1)?.[1]).toMatchObject({ visible: true, occluded: true, preserveBackground: true, width: 691 });
    expect(api.setViewport.mock.calls.every(([, viewport]) => viewport.visible)).toBe(true);
    await act(async () => root.render(<NativeStreamingPane api={api} pane={pane} epoch={7} privacy inputBlocked onError={() => {}} />));
    expect(api.setViewport.mock.calls.at(-1)?.[1]).toMatchObject({ visible: false, suspendAudio: true });
  });
  it("releases a divider drag on Alt-Tab without changing the user's privacy state", async () => {
    const api = fake(); api.snapshot.mockResolvedValue(success(multiple()));
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue(new DOMRect(0, 0, 800, 600));
    await act(async () => root.render(<WorkspaceFixture runtime="tauri" privacy={false} onLeave={() => {}} api={api} />));
    const event = new MouseEvent("pointerdown", { clientX: 480, button: 0, bubbles: true });
    Object.defineProperty(event, "pointerId", { value: 1 });
    await act(async () => container.querySelector('[role="separator"]')!.dispatchEvent(event));
    expect(container.querySelector('.mado-surfaces')).toHaveClass('is-resizing');
    expect(api.setViewport.mock.calls.slice(-8).every(([, viewport]) => viewport.suspendAudio !== true)).toBe(true);
    await act(async () => window.dispatchEvent(new Event("blur")));
    expect(container.querySelector('.mado-surfaces')).not.toHaveClass('is-resizing');
    expect(api.close).not.toHaveBeenCalled(); expect(api.configure).not.toHaveBeenCalled();
  });
  it("leaves each native player's audio and S controls in charge without an outside duplicate", async () => {
    const api = fake(); const state = multiple(); api.snapshot.mockResolvedValue(success(state));
    await act(async () => root.render(<WorkspaceFixture runtime="tauri" privacy={false} onLeave={() => {}} api={api} />));
    expect(container.querySelector('.mado-audio, .mado-pane-name')).toBeNull();
    await act(async () => window.dispatchEvent(new KeyboardEvent("keydown", { key: "s", bubbles: true })));
    expect(api.requestControl).not.toHaveBeenCalled();
    expect(api.setPaneAudio).not.toHaveBeenCalled(); expect(api.setAudio).not.toHaveBeenCalled();
    await act(async () => root.render(<WorkspaceFixture runtime="tauri" privacy onLeave={() => {}} api={api} />));
    expect(container).not.toHaveTextContent("도서관 1");
  });
  it("does not show another confirmation for a native per-pane recording intent", async () => {
    const api = fake(); const state = multiple();
    state.pendingControl = { paneId: "video-1", id: "record-second", action: "record_start", channelId: B, expiresAt: Date.now() + 20000 };
    api.snapshot.mockResolvedValue(success(state));
    await act(async () => root.render(<WorkspaceFixture runtime="tauri" privacy={false} onLeave={() => {}} api={api} />));
    expect(container.querySelector('[role="alertdialog"]')).toBeNull();
    expect(api.confirmControl).not.toHaveBeenCalled();
  });
  it("normalizes only exact official channel identifiers and refuses credentials/query/foreign URLs", () => {
    expect(normalizeMultiviewChannel(A.toUpperCase())).toBe(A);
    expect(normalizeMultiviewChannel(` https://chzzk.naver.com/${A}/ `)).toBe(A);
    expect(normalizeMultiviewChannel(`https://chzzk.naver.com/live/${A}/chat`)).toBe(A);
    for (const value of [`http://chzzk.naver.com/live/${A}`, `https://chzzk.naver.com.evil.test/live/${A}`, `https://user@chzzk.naver.com/live/${A}`, `https://chzzk.naver.com/live/${A}?extra=1`, "abc"]) expect(normalizeMultiviewChannel(value)).toBeNull();
  });
  it("creates one video and four chat panes without hidden extra video entries", () => {
    expect(multiviewEntries([A, B, C, D], "chats", 1)).toEqual([
      { channelId: A, video: false, chat: true }, { channelId: B, video: true, chat: true },
      { channelId: C, video: false, chat: true }, { channelId: D, video: false, chat: true },
    ]);
    expect(typeof multiviewEntries([A, A], "paired", 0)).toBe("string");
    expect(typeof multiviewEntries([A, "", C], "chats", 1)).toBe("string");
    expect(typeof multiviewEntries([], "paired", 0)).toBe("string");
  });
  it("does not open streams on mount and requires explicit valid configuration", async () => {
    const api = fake(); await act(async () => root.render(<WorkspaceFixture runtime="tauri" privacy={false} onLeave={() => {}} api={api} />));
    expect(api.configure).not.toHaveBeenCalled();
    expect(button("시청 시작")).toBeDisabled(); expect(api.configure).not.toHaveBeenCalled();
    await enter(0, A); await act(async () => button("시청 시작").click());
    expect(api.configure).toHaveBeenCalledWith([{ channelId: A, video: true, chat: true }]);
    expect(container.querySelectorAll(".mado-native-slot")).toHaveLength(2);
  });
  it("keeps a denied recording-active transition visible instead of pretending to open panes", async () => {
    const api = fake(); api.configure.mockResolvedValue({ ok: false, error: { code: "RECORDING_ACTIVE", message: "녹화를 먼저 중지해 주세요.", retryable: true } });
    await act(async () => root.render(<WorkspaceFixture runtime="tauri" privacy={false} onLeave={() => {}} api={api} />));
    await enter(0, A); await act(async () => button("시청 시작").click());
    expect(container.querySelector('[role="alert"]')).toHaveTextContent("녹화를 먼저 중지");
    expect(container.querySelectorAll(".mado-native-slot")).toHaveLength(0);
  });
  it("closes the expected epoch before leaving and hides panes for privacy", async () => {
    const api = fake(), leave = vi.fn(); api.snapshot.mockResolvedValue(success(active()));
    await act(async () => root.render(<WorkspaceFixture runtime="tauri" privacy onLeave={leave} api={api} />));
    expect(api.setViewport.mock.calls.every(([, viewport]) => viewport.visible === false && viewport.epoch === 7)).toBe(true);
    await act(async () => root.render(<WorkspaceFixture runtime="tauri" privacy={false} onLeave={leave} api={api} />));
    await act(async () => button("연결 설정").click());
    await act(async () => button("일반모드").click());
    expect(api.close).not.toHaveBeenCalled();
    await act(async () => button("시청 시작").click());
    expect(api.close).toHaveBeenCalledWith(7); expect(leave).toHaveBeenCalledOnce();
    expect(invoke).toHaveBeenCalledWith("chzzk_browser_open", { input: A });
  });
  it("binds audio and close to the native epoch and assigns unique viewport sequences", async () => {
    vi.mocked(invoke).mockResolvedValue(success(undefined));
    const api = createMultiviewApi("tauri");
    await api.setAudio(A, 7); expect(invoke).toHaveBeenCalledWith("chzzk_multiview_set_audio", { channelId: A, epoch: 7 });
    await api.close(7); expect(invoke).toHaveBeenCalledWith("chzzk_multiview_close", { epoch: 7 });
    const viewport = { x: 0, y: 0, width: 100, height: 100, visible: true, epoch: 7 };
    await api.setViewport("mado-7-a-video", viewport); await api.setViewport("mado-7-a-video", { ...viewport, visible: false });
    const calls = vi.mocked(invoke).mock.calls.filter(([name]) => name === "chzzk_multiview_set_viewport");
    const sequences = calls.map(([, args]) => (args as { viewport: { requestSequence: number } }).viewport.requestSequence);
    expect(sequences).toHaveLength(2);
    expect(sequences[1]!).toBeGreaterThan(sequences[0]!);
  });
  it("does not let a delayed initial snapshot replace a newer configured layout", async () => {
    const api = fake(), initial = deferred<Awaited<ReturnType<MultiviewApi["snapshot"]>>>();
    api.snapshot.mockReturnValue(initial.promise);
    await act(async () => root.render(<WorkspaceFixture runtime="tauri" privacy={false} onLeave={() => {}} api={api} />));
    await enter(0, A); await act(async () => button("시청 시작").click());
    expect(container.querySelectorAll(".mado-native-slot")).toHaveLength(2);
    await act(async () => initial.resolve(success(emptyMultiview())));
    expect(container.querySelectorAll(".mado-native-slot")).toHaveLength(2);
    expect(container.querySelector(".connection-setup")).toBeNull();
  });
  it("restores channel drafts and the selected video when reopening an existing one-video layout", async () => {
    const api = fake();
    api.snapshot.mockResolvedValue(success({ active: true, epoch: 9, audioOwner: B, panes: [A, B, C, D].flatMap((channelId) => [
      ...(channelId === B ? [{ paneId: `restored-${channelId}-video`, channelId, kind: "video" as const, status: "ready" }] : []),
      { paneId: `restored-${channelId}-chat`, channelId, kind: "chat" as const, status: "ready" },
    ]) }));
    await act(async () => root.render(<WorkspaceFixture runtime="tauri" privacy={false} onLeave={() => {}} api={api} />));
    expect(container.querySelector(".mado-surfaces")).toHaveClass("is-chats");
    await act(async () => button("연결 설정").click());
    expect([0, 1, 2, 3].map((index) => container.querySelector<HTMLInputElement>(`#mado-channel-${index}`)!.value)).toEqual([A, B, C, D]);
    expect(button("선택 보기")).toHaveAttribute("aria-pressed", "true");
    expect(container.querySelector<HTMLSelectElement>(".connection-setup select")?.value).toBe("1");
    expect(api.configure).not.toHaveBeenCalled();
  });
  it("styles the one-video selector as a connection field and only applies its choice on connect", async () => {
    const api = fake(); api.snapshot.mockResolvedValue(success(multiple(true)));
    await act(async () => root.render(<WorkspaceFixture runtime="tauri" privacy={false} onLeave={() => {}} api={api} />));
    await act(async () => button("연결 설정").click());
    const panes = [...container.querySelectorAll(".mado-native-slot")];
    const select = container.querySelector<HTMLSelectElement>(".mado-broadcast-select select")!;
    expect(select).toHaveAccessibleName("영상 방송");
    expect(select.value).toBe("0");
    expect([...select.options].map(option => option.textContent)).toEqual(["도서관 1", "도서관 2", "도서관 3", "도서관 4"]);
    expect(select.nextElementSibling).toHaveAttribute("aria-hidden", "true");
    await act(async () => { select.value = "2"; select.dispatchEvent(new Event("change", { bubbles: true })); });
    expect(select.value).toBe("2");
    expect([...container.querySelectorAll(".mado-native-slot")]).toEqual(panes);
    expect(api.configure).not.toHaveBeenCalled(); expect(api.close).not.toHaveBeenCalled();
    await act(async () => button("시청 시작").click());
    expect(api.configure).toHaveBeenCalledExactlyOnceWith([
      { channelId: A, video: false, chat: true }, { channelId: B, video: false, chat: true },
      { channelId: C, video: true, chat: true }, { channelId: D, video: false, chat: true },
    ]);
  });
  it("does not overwrite a channel draft edited before the initial snapshot arrives", async () => {
    const api = fake(), initial = deferred<Awaited<ReturnType<MultiviewApi["snapshot"]>>>();
    api.snapshot.mockReturnValue(initial.promise);
    await act(async () => root.render(<WorkspaceFixture runtime="tauri" privacy={false} onLeave={() => {}} api={api} />));
    await enter(0, B);
    await act(async () => initial.resolve(success(active())));
    expect(container.querySelector<HTMLInputElement>("#mado-channel-0")!.value).toBe(B);
    expect(container.querySelector("#mado-channel-0")).not.toBeNull();
    expect(api.configure).not.toHaveBeenCalled();
  });
  it("closes a configure result that arrives after unmount without changing the new workspace", async () => {
    const oldApi = fake(), newerApi = fake(), configured = deferred<Awaited<ReturnType<MultiviewApi["configure"]>>>();
    oldApi.configure.mockReturnValue(configured.promise);
    newerApi.snapshot.mockResolvedValue(success({ ...active(), epoch: 9 }));
    await act(async () => root.render(<WorkspaceFixture key="old" runtime="tauri" privacy={false} onLeave={() => {}} api={oldApi} />));
    await enter(0, A); await act(async () => button("시청 시작").click());
    await act(async () => root.render(<WorkspaceFixture key="new" runtime="tauri" privacy={false} onLeave={() => {}} api={newerApi} />));
    await act(async () => configured.resolve(success(active())));
    expect(oldApi.close).toHaveBeenCalledWith(7);
    expect(newerApi.close).not.toHaveBeenCalled();
    expect(container.querySelectorAll(".mado-native-slot")).toHaveLength(2);
    expect(newerApi.setViewport.mock.calls.every(([, viewport]) => viewport.epoch === 9)).toBe(true);
  });
  it("ignores a late leave completion after unmount instead of leaving a replacement workspace", async () => {
    const oldApi = fake(), newerApi = fake(), closed = deferred<Awaited<ReturnType<MultiviewApi["close"]>>>(), leave = vi.fn();
    oldApi.snapshot.mockResolvedValue(success(active())); oldApi.close.mockReturnValue(closed.promise);
    newerApi.snapshot.mockResolvedValue(success({ ...active(), epoch: 9 }));
    await act(async () => root.render(<WorkspaceFixture key="old" runtime="tauri" privacy={false} onLeave={leave} api={oldApi} />));
    await act(async () => button("연결 설정").click());
    await act(async () => button("일반모드").click());
    await act(async () => button("시청 시작").click());
    await act(async () => root.render(<WorkspaceFixture key="new" runtime="tauri" privacy={false} onLeave={leave} api={newerApi} />));
    await act(async () => closed.resolve(success(emptyMultiview())));
    expect(leave).not.toHaveBeenCalled();
    expect(button("연결 설정")).toBeEnabled();
    expect(container.querySelectorAll(".mado-native-slot")).toHaveLength(2);
  });
  it("resets a pending action on API replacement and ignores its stale screenshot response", async () => {
    const oldApi = fake(), newerApi = fake(), capture = deferred<Awaited<ReturnType<MultiviewApi["requestControl"]>>>();
    oldApi.snapshot.mockResolvedValue(success(active())); oldApi.requestControl.mockReturnValue(capture.promise);
    newerApi.snapshot.mockResolvedValue(success({ ...active(), epoch: 9, panes: active().panes.map((pane) => ({ ...pane, audioEnabled: true })) }));
    await act(async () => root.render(<WorkspaceFixture runtime="tauri" privacy={false} onLeave={() => {}} api={oldApi} />));
    await act(async () => window.dispatchEvent(new KeyboardEvent("keydown", { key: "s", bubbles: true })));
    expect(oldApi.requestControl).toHaveBeenCalledOnce();
    await act(async () => button("연결 설정").click());
    expect(button("일반모드")).toBeDisabled();
    await act(async () => root.render(<WorkspaceFixture runtime="tauri" privacy={false} onLeave={() => {}} api={newerApi} />));
    await act(async () => button("연결 설정").click());
    expect(button("일반모드")).toBeEnabled();
    await act(async () => capture.resolve(success(active())));
    expect(container.querySelectorAll(".mado-native-slot")).toHaveLength(2);
    expect(container.querySelector('[role="alertdialog"]')).toBeNull();
    expect(newerApi.setViewport.mock.calls.every(([, viewport]) => viewport.epoch === 9 || viewport.visible === false)).toBe(true);
  });
  it("retries transient multiview viewport contention twice without a permanent retry loop", async () => {
    const api = fake(); api.snapshot.mockResolvedValue(success(active()));
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue(new DOMRect(10, 10, 400, 300));
    api.setViewport.mockImplementation(async (_paneId, viewport) => viewport.visible
      ? { ok: false, error: { code: "MULTIVIEW_BUSY", message: "영역 연결을 다시 확인해 주세요.", retryable: true } } : success(undefined));
    await act(async () => root.render(<WorkspaceFixture runtime="tauri" privacy={false} onLeave={() => {}} api={api} />));
    await act(async () => vi.advanceTimersByTimeAsync(500));
    expect(api.setViewport.mock.calls.filter(([, viewport]) => viewport.visible && !viewport.occluded)).toHaveLength(6);
    await act(async () => vi.advanceTimersByTimeAsync(10000));
    // The resulting trusted error popup is a new masking purpose, which has
    // its own bounded retries and fail-closed hides, never a normal re-show.
    const total = api.setViewport.mock.calls.length;
    expect(api.setViewport.mock.calls.filter(([, viewport]) => viewport.visible && !viewport.occluded)).toHaveLength(6);
    expect(api.setViewport.mock.calls.filter(([, viewport]) => viewport.occluded).length).toBeLessThanOrEqual(6);
    await act(async () => vi.advanceTimersByTimeAsync(10000));
    expect(api.setViewport).toHaveBeenCalledTimes(total);
    expect(container.querySelector('[role="alert"]')).toHaveTextContent("영역 연결을 다시 확인");
  });
  it("sends privacy hides immediately and never retries a late visible failure", async () => {
    const api = fake(), visible = deferred<Awaited<ReturnType<MultiviewApi["setViewport"]>>>();
    api.snapshot.mockResolvedValue(success(active()));
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue(new DOMRect(10, 10, 400, 300));
    api.setViewport.mockImplementation(async (_paneId, viewport) => viewport.visible ? visible.promise : success(undefined));
    await act(async () => root.render(<WorkspaceFixture runtime="tauri" privacy={false} onLeave={() => {}} api={api} />));
    expect(api.setViewport.mock.calls.filter(([, viewport]) => viewport.visible)).toHaveLength(2);
    await act(async () => root.render(<WorkspaceFixture runtime="tauri" privacy onLeave={() => {}} api={api} />));
    expect(api.setViewport.mock.calls.slice(-2).every(([, viewport]) => !viewport.visible)).toBe(true);
    const afterHide = api.setViewport.mock.calls.length;
    await act(async () => visible.resolve({ ok: false, error: { code: "MULTIVIEW_BUSY", message: "stale visible", retryable: true } }));
    await act(async () => vi.advanceTimersByTimeAsync(10000));
    expect(api.setViewport).toHaveBeenCalledTimes(afterHide);
    expect(container.querySelector('[role="alert"]')).toBeNull();
  });
  it.each(["privacy", "unmount"] as const)("does not revive panes from an old animation frame after immediate popup masking and %s", async (transition) => {
    const api = fake(), state = multiple(); api.snapshot.mockResolvedValue(success(state));
    const pendingFrames = new Map<number, FrameRequestCallback>();
    let nextFrame = 0;
    vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
      const id = ++nextFrame; pendingFrames.set(id, callback); return id;
    });
    const cancelFrame = vi.fn((id: number) => { pendingFrames.delete(id); });
    vi.stubGlobal("cancelAnimationFrame", cancelFrame);
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue(new DOMRect(10, 10, 400, 300));
    await act(async () => root.render(<WorkspaceFixture runtime="tauri" privacy={false} onLeave={() => {}} api={api} />));
    expect(container.querySelectorAll(".mado-native-slot")).toHaveLength(8);
    expect(api.setViewport.mock.calls.slice(-8).every(([, viewport]) => viewport.visible && !viewport.occluded)).toBe(true);
    await act(async () => window.dispatchEvent(new Event("resize")));
    const savedFrames = [...pendingFrames.entries()];
    expect(savedFrames).toHaveLength(8);
    const modal = document.createElement("div"), surface = document.createElement("div");
    modal.setAttribute("role", "alertdialog"); modal.dataset.nativePreserveVideo = "true";
    surface.dataset.nativeDialogSurface = "true";
    Object.defineProperty(surface, "getBoundingClientRect", { value: () => new DOMRect(80, 60, 120, 100) });
    modal.append(surface);
    try {
      api.setViewport.mockClear();
      await act(async () => document.body.append(modal));
      // Popup mutation must preempt a pending normal frame without shrinking
      // any of the four video/chat pairs or abandoning its cancellation ID.
      expect(api.setViewport.mock.calls).toHaveLength(8);
      for (const [, viewport] of api.setViewport.mock.calls) {
        expect(viewport).toMatchObject({ visible: true, occluded: true, preserveBackground: true, width: 400, height: 300 });
        expect(viewport.occlusions).toHaveLength(1);
      }
      for (const [id] of savedFrames) expect(cancelFrame).toHaveBeenCalledWith(id);
      await act(async () => root.render(transition === "privacy"
        ? <WorkspaceFixture runtime="tauri" privacy onLeave={() => {}} api={api} /> : null));
      expect(api.setViewport.mock.calls.slice(-8).every(([, viewport]) => !viewport.visible)).toBe(true);
      await act(async () => modal.remove());
      api.setViewport.mockClear();
      // Force canceled callbacks to run as if already queued by the browser.
      // An old non-private closure must not send even a masked visible update.
      await act(async () => { for (const [, callback] of savedFrames) callback(32); });
      expect(api.setViewport).not.toHaveBeenCalled();
      expect(api.close).not.toHaveBeenCalled(); expect(api.confirmControl).not.toHaveBeenCalled();
    } finally { modal.remove(); }
  });
  it("masks every pane for a trusted modal without hiding or stopping media", async () => {
    const api = fake(); api.snapshot.mockResolvedValue(success(active()));
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue(new DOMRect(10, 10, 400, 300));
    await act(async () => root.render(<WorkspaceFixture runtime="tauri" privacy={false} onLeave={() => {}} api={api} />));
    api.setViewport.mockClear();
    const modal = document.createElement("div"); modal.setAttribute("role", "alertdialog");
    try {
      await act(async () => document.body.append(modal));
      expect(api.setViewport.mock.calls).toHaveLength(2);
      expect(api.setViewport.mock.calls.every(([, viewport]) => viewport.visible && viewport.occluded && viewport.width === 400 && viewport.height === 300)).toBe(true);
      await act(async () => { modal.remove(); await vi.advanceTimersByTimeAsync(20); });
      expect(api.setViewport.mock.calls.slice(-2).every(([, viewport]) => viewport.visible && !viewport.occluded)).toBe(true);
      expect(api.setViewport.mock.calls.every(([, viewport]) => viewport.visible)).toBe(true);
      expect(api.close).not.toHaveBeenCalled(); expect(api.setAudio).not.toHaveBeenCalled(); expect(api.setPaneAudio).not.toHaveBeenCalled();
    } finally { modal.remove(); }
  });
  it("does not reclip or reconnect idle players when pointer activity and snapshot polls repeat", async () => {
    const api = fake(); api.snapshot.mockResolvedValue(success(active()));
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue(new DOMRect(10, 10, 400, 300));
    await act(async () => root.render(<WorkspaceFixture runtime="tauri" privacy={false} unifiedLive onLeave={() => {}} api={api} />));
    await act(async () => vi.advanceTimersByTimeAsync(32));
    api.setViewport.mockClear();
    for (let cycle = 0; cycle < 3; cycle++) {
      await act(async () => {
        window.dispatchEvent(new MouseEvent('pointermove'));
        for (const handler of presentationEvents) handler({ payload: { action: 'pointer_activity', channelId: A } });
        container.querySelector('.live-bottom-handle')!.dispatchEvent(new Event('animationiteration', { bubbles: true }));
        await vi.advanceTimersByTimeAsync(4000);
      });
    }
    expect(api.setViewport).not.toHaveBeenCalled();
    expect(api.configure).not.toHaveBeenCalled(); expect(api.close).not.toHaveBeenCalled();
  });
  it("installs nonmodal panel holes immediately while an older visible update is still pending", async () => {
    const api = fake(), visible = deferred<Awaited<ReturnType<MultiviewApi["setViewport"]>>>();
    api.snapshot.mockResolvedValue(success(active()));
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue(new DOMRect(10, 10, 400, 300));
    api.setViewport.mockImplementation(async (_paneId, viewport) => viewport.visible && !viewport.preserveBackground ? visible.promise : success(undefined));
    await act(async () => root.render(<WorkspaceFixture runtime="tauri" privacy={false} unifiedLive onLeave={() => {}} api={api} />));
    expect(api.setViewport).toHaveBeenCalledTimes(2);
    await act(async () => button("방송 보기").click());
    const protectedUpdates = api.setViewport.mock.calls.filter(([, viewport]) => viewport.preserveBackground);
    expect(protectedUpdates).toHaveLength(2);
    for (const [, viewport] of protectedUpdates) {
      expect(viewport).toMatchObject({ visible: true, occluded: false, preserveBackground: true });
      expect(viewport.occlusions).toHaveLength(1);
    }
    await act(async () => { button("연결 설정 닫기").click(); await vi.advanceTimersByTimeAsync(20); });
    await act(async () => visible.resolve(success(undefined)));
    expect(container.querySelector('.mado-connection-panel')).toBeNull();
    expect(container.querySelector('.live-bottom-handle[data-native-overlay=true]')).not.toBeNull();
    expect(api.setViewport.mock.calls.slice(-2).every(([, viewport]) => viewport.visible && !viewport.occluded && viewport.preserveBackground)).toBe(true);
    expect(api.close).not.toHaveBeenCalled();
  });
  it.each(["modal", "panel"] as const)("hard-hides after a %s transport rejection before reporting its safe error", async kind => {
    const api = fake(); api.snapshot.mockResolvedValue(success(active()));
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue(new DOMRect(10, 10, 400, 300));
    await act(async () => root.render(<WorkspaceFixture runtime="tauri" privacy={false} onLeave={() => {}} api={api} />));
    const rejected = new Set<string>();
    api.setViewport.mockClear().mockImplementation(async (paneId, viewport) => {
      if ((viewport.occluded || viewport.preserveBackground) && !rejected.has(paneId)) { rejected.add(paneId); throw new Error("secret transport detail"); }
      return success(undefined);
    });
    const modal = document.createElement("div"); modal.setAttribute("role", "alertdialog");
    if (kind === "panel") {
      modal.setAttribute("role", "dialog"); modal.setAttribute("aria-modal", "false");
      modal.dataset.nativeOverlay = "true"; modal.dataset.nativePreserveVideo = "true"; modal.dataset.nativeInteractiveBackground = "true";
      const surface = document.createElement("div"); surface.dataset.nativeDialogSurface = "true"; modal.append(surface);
    }
    try {
      await act(async () => document.body.append(modal));
      expect(api.setViewport.mock.calls.filter(([, viewport]) => !viewport.visible)).toHaveLength(2);
      expect(container.querySelector('[role="alert"]')).toHaveTextContent("시청 영역을 표시하지 못했습니다");
      expect(container).not.toHaveTextContent("secret transport detail");
    } finally { modal.remove(); }
  });
  it("does not let late modal failure hide a newer queued restoration", async () => {
    const api = fake(), visible = deferred<Awaited<ReturnType<MultiviewApi["setViewport"]>>>(), masked = deferred<Awaited<ReturnType<MultiviewApi["setViewport"]>>>();
    api.snapshot.mockResolvedValue(success(active()));
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue(new DOMRect(10, 10, 400, 300));
    api.setViewport.mockImplementation(async (_paneId, viewport) => viewport.occluded ? masked.promise : viewport.visible ? visible.promise : success(undefined));
    await act(async () => root.render(<WorkspaceFixture runtime="tauri" privacy={false} onLeave={() => {}} api={api} />));
    const modal = document.createElement("div"); modal.setAttribute("role", "alertdialog");
    try {
      await act(async () => document.body.append(modal));
      expect(api.setViewport.mock.calls.filter(([, viewport]) => viewport.occluded)).toHaveLength(2);
      await act(async () => { modal.remove(); await vi.advanceTimersByTimeAsync(20); });
      await act(async () => masked.resolve({ ok: false, error: { code: "VIEWPORT_CLIP_FAILED", message: "stale mask", retryable: true } }));
      await act(async () => visible.resolve(success(undefined)));
      await act(async () => vi.advanceTimersByTimeAsync(1000));
      expect(api.setViewport.mock.calls.every(([, viewport]) => viewport.visible)).toBe(true);
      expect(api.setViewport.mock.calls.slice(-2).every(([, viewport]) => !viewport.occluded)).toBe(true);
      expect(container.querySelector('[role="alert"]')).toBeNull();
    } finally { modal.remove(); }
  });
});
