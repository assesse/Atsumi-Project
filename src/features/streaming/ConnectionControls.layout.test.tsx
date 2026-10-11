import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi } from "vitest";
import { ConnectionSetup } from "./ConnectionSetup";
import { AutoRecordingManagerDialog } from "./AutoRecordingManagerDialog";
import { auditLayouts, layoutBrowser } from "../../test/offlineLayoutBrowser";
import appCss from "../../styles.css?raw";
import officialCss from "./OfficialBrowserPanel.css?raw";
import pickerCss from "./LiveChannelPicker.css?raw";
import autoCss from "./AutoRecordingPanel.css?raw";
import workspaceCss from "./StreamingWorkspace.css?raw";
import madoCss from "./MadoWorkspace.css?raw";
import { MadoWorkspace } from "./MadoWorkspace";
import { emptyMultiview, type MultiviewApi, type MultiviewSnapshot } from "../../api/multiview";
import { createOfficialBrowserApi } from "../../api/officialBrowser";
import { LiveChannelPicker } from "./LiveChannelPicker";
import type { LiveChannelsApi } from "../../api/liveChannels";
const success = <T,>(data: T) => ({ ok: true as const, data });

describe.skipIf(!layoutBrowser)("live connection control geometry", () => {
  it.each([[960, 640], [1440, 900]])("centers icons and keeps each borderless clear inside its input at %ix%i", async (width, height) => {
    const container = document.createElement('div'); document.body.append(container);
    const root = createRoot(container), frames: { name: string; html: string }[] = [];
    const snapshot = async () => success({ channels: [], captureChat: true, error: null });
    const autoApi = { snapshot, add: snapshot, update: snapshot };
    try {
      await act(async () => root.render(<ConnectionSetup mode="mado" unified onMode={vi.fn()} modeDisabled={false}
        inputs={['a'.repeat(32), '']} onInput={vi.fn()} onAddChannel={vi.fn()} onRemoveChannel={vi.fn()}
        disabled={false} pending={false} onConnect={vi.fn()} auth="signed_out" accountDisabled={false}
        onLogin={vi.fn()} onLogout={vi.fn()} onGrid={vi.fn()} gridDisabled gridStatus="" onInstaller={vi.fn()} installerDisabled
        runtime="browser-mock" privacy={false} autoApi={autoApi} />));
      frames.push({ name: 'channel-inputs', html: `<main class="streaming-workspace">${container.innerHTML}</main>` });
      // A narrow panel switches to one column; the limit is four visual rows,
      // not four channels. Exercise both short and overflowing directories.
      for (const panelWidth of [380, 820]) for (const count of [2, 4, 8, 10]) {
        const favorites = Array.from({ length: count }, (_, index) => ({ channelId: index.toString(16).padStart(32, '0'), channelName: `즐겨찾기 채널 ${index + 1}` }));
        const channelsApi: LiveChannelsApi = { snapshot: async () => success(favorites), set: async () => success(favorites), profile: async () => success({ channelName: '긴 이름의 방송 채널', image: null, liveStatus: 'live' }) };
        await act(async () => root.render(<LiveChannelPicker key={`${panelWidth}-${count}`} runtime="tauri" privacy={false} inputs={['']} multiple disabled={false} onInputs={vi.fn()} api={channelsApi} autoApi={autoApi} />));
        expect(container.querySelectorAll('.live-channel-row')).toHaveLength(count);
        frames.push({ name: `favorites-${panelWidth}-${count}`, html: `<main class="streaming-workspace" style="width:${panelWidth}px">${container.innerHTML}</main>` });
      }
      for (const watching of [false, true]) {
        const state: MultiviewSnapshot = { ...emptyMultiview(), active: watching, panes: watching ? ['a', 'b'].flatMap((letter, index) => [
          { paneId: `${letter}-video`, channelId: letter.repeat(32), channelName: `긴 채널 이름 ${index + 1}`, kind: 'video' as const, status: 'ready' },
          { paneId: `${letter}-chat`, channelId: letter.repeat(32), channelName: `긴 채널 이름 ${index + 1}`, kind: 'chat' as const, status: 'ready' },
        ]) : [] };
        const get = async () => success(state);
        const api: MultiviewApi = { runtime: 'tauri', snapshot: get, configure: get, close: get, setAudio: get, setPaneAudio: get, requestControl: get, confirmControl: get, ackUiAction: get, setViewport: async () => success(undefined) };
        await act(async () => root.render(<MadoWorkspace key={String(watching)} runtime="browser-mock" api={api} officialApi={createOfficialBrowserApi('browser-mock')} privacy={false} unifiedLive connectionRequest={1} onLeave={vi.fn()} />));
        for (const mode of ['paired', 'chats']) {
          if (mode === 'chats') await act(async () => [...container.querySelectorAll('button')].find(button => button.textContent === '선택 보기')!.click());
          frames.push({ name: `live-actions-${watching}-${mode}`, html: `<main class="streaming-workspace">${container.innerHTML}</main>` });
        }
      }
      await act(async () => root.render(<AutoRecordingManagerDialog runtime="browser-mock" privacy={false} api={autoApi} onChange={vi.fn()} onClose={vi.fn()} />));
      const dialog = document.querySelector('.auto-record-manager-dialog');
      frames.push({ name: 'recording-manager', html: dialog!.outerHTML });
      const results = await auditLayouts(frames, `${appCss}\n${workspaceCss}\n${officialCss}\n${madoCss}\n${pickerCss}\n${autoCss}\nbody{padding:16px}`, width, height, 'live-controls');
      expect(results.flatMap(result => result.issues.map(issue => `${result.name}: ${issue}`))).toEqual([]);
    } finally { await act(async () => root.unmount()); container.remove(); }
  }, 60000);
});
