import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { SideRail } from "../../components/SideRail";
import { LiveEdgePanel } from "./LiveEdgePanel";
import { auditLayouts, layoutBrowser } from "../../test/offlineLayoutBrowser";
import appCss from "../../styles.css?raw";
import workspaceCss from "./StreamingWorkspace.css?raw";
import madoCss from "./MadoWorkspace.css?raw";
import edgeCss from "./LiveEdgePanel.css?raw";

afterEach(() => vi.unstubAllGlobals());
describe.skipIf(!layoutBrowser)('live edge controls in small windows', () => {
  it.each([[960, 640], [1440, 900]])('keeps the actual sidebar and toolbar controls reachable at %ix%i', async (width, height) => {
    vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true);
    const host = document.createElement('div'), root = createRoot(host), noop = () => {};
    document.body.append(host);
    try {
      await act(async () => root.render(<div id="root"><div className="app-shell streaming-shell is-live-immersive">
        <LiveEdgePanel enabled edge="left" label="메뉴"><SideRail source="chzzk" view="live" collapsed={false} autoFindCount={0} attentionCount={0} onNavigate={noop} onSourceChange={noop} onToggle={noop} onSettings={noop} privacyMode={false} onPrivacyModeToggle={noop} /></LiveEdgePanel>
        <main className="streaming-workspace is-official-view"><section className="mado-workspace is-immersive">
          <LiveEdgePanel enabled edge="top" label="라이브 도구"><header className="mado-toolbar"><strong>라이브</strong><button>방송 보기</button><button>시청 종료</button><button>위아래 배치</button></header></LiveEdgePanel>
          <div className="mado-surfaces"><div className="mado-native-slot">영상</div><div /><div className="mado-native-slot">채팅</div></div>
        </section></main>
      </div></div>));
      const scenes = [{ name: 'edges-closed', html: host.innerHTML }];
      await act(async () => host.querySelector<HTMLButtonElement>('.edge-left>.live-edge-trigger')!.click());
      scenes.push({ name: 'sidebar-open', html: host.innerHTML });
      await act(async () => host.querySelector<HTMLButtonElement>('.edge-top>.live-edge-trigger')!.click());
      scenes.push({ name: 'sidebar-and-toolbar-open', html: host.innerHTML });
      const results = await auditLayouts(scenes, [appCss, workspaceCss, madoCss, edgeCss].join('\n'), width, height, 'live-edges');
      expect(results.flatMap(result => result.issues.map(issue => `${result.name}: ${issue}`))).toEqual([]);
    } finally { await act(async () => root.unmount()); host.remove(); }
  }, 60000);
});
