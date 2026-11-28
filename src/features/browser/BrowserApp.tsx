import { useEffect, useRef, useState, type ReactNode } from "react";
import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { FluentIcon } from "../../components/FluentIcon";
import atsumiLogo from "../../assets/atsumi.svg";
import { BrowserLibrary, type LibraryKind, type LibraryPage } from "./BrowserLibrary";
export type BrowserPreferences = { savePasswords: boolean; autofill: boolean; restoreTabs: boolean };

export type BrowserProfile = { id: string; name: string; countryHint: string | null };
export type BrowserSnapshot = {
  tabs: { id: string; url: string; title: string; loading: boolean; canGoBack?: boolean; canGoForward?: boolean }[];
  active: string | null;
  profiles: BrowserProfile[];
  network: { enabled: boolean; fragment: boolean; phase: string; ip: string | null; country: string | null; profile: string | null; profileId?: string | null; error: string | null };
  downloads: number;
  notice: string | null;
  preferences: BrowserPreferences;
  passwordSavingAvailable: boolean;
  maximized: boolean;
  recentlyClosed: number;
};
export const initialBrowserState: BrowserSnapshot = {
  tabs: [], active: null, profiles: [],
  network: { enabled: false, fragment: false, phase: "direct", ip: null, country: null, profile: null, error: null },
  downloads: 0, notice: null,
  preferences: { savePasswords: true, autofill: true, restoreTabs: true }, passwordSavingAvailable: false, maximized: false, recentlyClosed: 0,
};
export const browserApi = {
  snapshot: () => invoke<BrowserSnapshot>("browser_snapshot"),
  window: (action: string) => invoke<void>("browser_window", { action }),
  preferences: (preferences: BrowserPreferences) => invoke<void>("browser_preferences", { preferences }),
  libraryQuery: (kind: string, query: string, offset: number) => invoke<LibraryPage>("browser_library_query", { kind, query, offset }),
  libraryUpdate: (kind: string, action: string, id?: string, url?: string, title?: string) => invoke<boolean>("browser_library_update", { kind, action, id, url, title }),
  tab: (action: string, id?: string, url?: string) => invoke<void>("browser_tab", { action, id, url }),
  network: (enabled: boolean, fragment: boolean, profileId?: string) => invoke<void>("browser_network", { enabled, fragment, profileId }),
  importConfig: () => invoke<void>("browser_import"),
  bounds: (x: number, y: number, width: number, height: number) => invoke<void>("browser_bounds", { x, y, width, height }),
  subscribe: (handler: (state: BrowserSnapshot) => void) => listen<BrowserSnapshot>("browser:state", e => handler(e.payload)),
  shortcuts: (handler: (key: string) => void) => listen<string>("browser:shortcut", e => handler(e.payload)),
};

export type BrowserApi = typeof browserApi;

function IconButton({ label, glyph, onClick, disabled, title, className = "" }: {
  label: string; glyph: string; onClick: () => void; disabled?: boolean; title?: string; className?: string;
}) {
  return <button type="button" className={"browser-icon-button " + className} aria-label={label} title={title ?? label} disabled={disabled} onClick={onClick}>
    <FluentIcon glyph={glyph}/>
  </button>;
}

function BrowserSymbol({ kind }: { kind: "shield" | "split" }) {
  return <svg className="browser-symbol" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.65" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
    {kind === "shield" ? <><path d="M12 3 20 6v6c0 4.5-4 7.6-8 9-4-1.4-8-4.5-8-9V6z"/><path d="m8.5 12 2.3 2.3 4.7-4.8"/></> : <><path d="M3 12h5m8 0h5M8 12l5-6h8M8 12l5 6h8"/><path d="m18 3 3 3-3 3m0 6 3 3-3 3"/></>}
  </svg>;
}

function NetworkSwitch({ label, checked, disabled, onClick }: {
  label: string; checked: boolean; disabled: boolean; onClick: () => void;
}) {
  return <button type="button" className="browser-switch" role="switch" aria-label={label} aria-checked={checked} disabled={disabled} onClick={onClick}>
    <span className="browser-switch-track"><span/></span><span>{checked ? "켜짐" : "꺼짐"}</span>
  </button>;
}

function SetupStep({ number, title, children }: { number: number; title: string; children: ReactNode }) {
  return <section className="browser-setup-step">
    <h3><span className="browser-step-number">{number}</span>{title}</h3>
    <div className="browser-step-body">{children}</div>
  </section>;
}

function countryLabel(code: string | null) {
  if (!code) return "국가 확인 중";
  return ({ JP: "일본", US: "미국", NL: "네덜란드", SG: "싱가포르", KR: "대한민국", CH: "스위스", CA: "캐나다", PL: "폴란드", NO: "노르웨이", RO: "루마니아", MX: "멕시코" } as Record<string, string>)[code] ?? code;
}

export function BrowserApp({ api = browserApi }: { api?: typeof browserApi }) {
  const [state, setState] = useState(initialBrowserState);
  const [address, setAddress] = useState("");
  const [settings, setSettings] = useState(false);
  const [library, setLibrary] = useState<LibraryKind | null>(null);
  const [bookmarked, setBookmarked] = useState(false);
  const [bookmarkRevision, setBookmarkRevision] = useState(0);
  const [guideExpanded, setGuideExpanded] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const input = useRef<HTMLInputElement>(null);
  const content = useRef<HTMLDivElement>(null);
  const current = state.tabs.find(tab => tab.id === state.active);
  const blank = !current || current.url === "about:blank";
  const network = state.network;
  const profiles = state.profiles ?? [];
  const selectedProfile = profiles.find(profile => profile.id === network.profileId);
  const orderedProfiles = [...profiles].sort((a, b) => countryLabel(a.countryHint).localeCompare(countryLabel(b.countryHint), "ko") || a.name.localeCompare(b.name));
  const connecting = network.phase === "connecting";
  const networkBusy = busy || connecting;
  const networkLocked = networkBusy || state.downloads > 0;
  const showGuide = !network.profile || guideExpanded;
  const connected = network.phase === "connected";
  const label = connected ? "VPN 연결됨 · " + countryLabel(network.country) : connecting ? "VPN 연결 중" : network.enabled ? "VPN 연결 차단됨" : "VPN 꺼짐";

  useEffect(() => {
    let alive = true;
    const off = api.subscribe(snapshot => { if (alive) setState(snapshot); });
    void api.snapshot().then(snapshot => { if (alive) setState(snapshot); }, e => { if (alive) setError(String(e)); });
    return () => { alive = false; void off.then(stop => stop()); };
  }, [api]);
  useEffect(() => { setAddress(blank ? "" : current?.url ?? ""); }, [blank, current?.url, current?.id]);

  const action = (work: Promise<unknown>) => { setError(null); void work.catch(e => setError(String(e))); };
  const go = (url: string) => {
    if (!url.trim()) return;
    setSettings(false); setLibrary(null);
    action(api.tab("go", undefined, url));
  };
  const newTab = (url?: string) => {
    setSettings(false); setLibrary(null);
    action(api.tab("new", undefined, url));
    if (!url) input.current?.focus();
  };
  const updateNetwork = async (enabled: boolean, fragment: boolean, profileId?: string) => {
    setBusy(true);
    setError(null);
    try { await (profileId ? api.network(enabled, fragment, profileId) : api.network(enabled, fragment)); }
    catch (e) { setError(String(e)); }
    finally { setBusy(false); }
  };
  const importConfig = async () => {
    setBusy(true);
    setError(null);
    try { await api.importConfig(); }
    catch (e) { setError(String(e)); }
    finally { setBusy(false); }
  };
  const openLibrary = (kind: LibraryKind) => { setSettings(false); setLibrary(kind); };
  useEffect(() => {
    let alive = true; setBookmarked(false);
    if (!blank && current) void api.libraryUpdate("bookmarks", "status", undefined, current.url).then(value => { if (alive) setBookmarked(value); }, () => {});
    return () => { alive = false; };
  }, [api, blank, current?.url, bookmarkRevision]);
  const toggleBookmark = async () => {
    if (blank || !current) return;
    const url = current.url; setError(null);
    try { await api.libraryUpdate("bookmarks", bookmarked ? "remove" : "add", bookmarked ? url : undefined, url, current.title); setBookmarkRevision(n => n + 1); }
    catch (e) { setError(String(e)); }
  };
  const shortcut = useRef((_key: string) => {});
  shortcut.current = key => {
    if (key === "address") { input.current?.focus(); input.current?.select(); }
    else if (key === "new") newTab();
    else if (key === "restore") { setSettings(false); setLibrary(null); action(api.tab("restore")); }
    else if (key === "history") openLibrary("history");
    else if (key === "bookmark") void toggleBookmark();
    else if (key === "close" && current) action(api.tab("close", current.id));
    else if (key === "next" || key === "previous") {
      const index = state.tabs.findIndex(tab => tab.id === state.active);
      const next = state.tabs[(index + (key === "previous" ? state.tabs.length - 1 : 1)) % state.tabs.length];
      if (next) { setSettings(false); setLibrary(null); action(api.tab("select", next.id)); }
    } else if (["reload", "back", "forward"].includes(key)) action(api.tab(key));
  };
  useEffect(() => {
    const subscription = api.shortcuts(key => shortcut.current(key));
    const keydown = (e: KeyboardEvent) => {
      let command: string | undefined;
      if (e.ctrlKey && !e.altKey) command = ({ l: "address", t: e.shiftKey ? "restore" : "new", h: "history", d: "bookmark", w: "close", Tab: e.shiftKey ? "previous" : "next" } as Record<string, string>)[e.key.length === 1 ? e.key.toLowerCase() : e.key];
      else if (e.key === "F5") command = "reload";
      else if (e.altKey) command = ({ ArrowLeft: "back", ArrowRight: "forward" } as Record<string, string>)[e.key];
      if (command) {
        e.preventDefault();
        // WebView2 routes native accelerators to browser:shortcut. Handling the
        // same keydown here too can move across two tabs and appear not to move.
        if (!isTauri()) shortcut.current(command);
      }
      else if (e.key === "Escape") { setSettings(false); setLibrary(null); }
    };
    window.addEventListener("keydown", keydown);
    return () => { window.removeEventListener("keydown", keydown); void subscription.then(stop => stop()); };
  }, [api]);
  useEffect(() => {
    const node = content.current;
    if (!node) return;
    let frame = 0;
    const resize = () => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => {
        const r = node.getBoundingClientRect();
        // A native child sits above HTML. Hide it completely while local setup is open.
        void api.bounds(r.x, r.y, settings || library || blank ? 0 : r.width, settings || library || blank ? 0 : r.height).catch(() => {});
      });
    };
    const observer = new ResizeObserver(resize);
    observer.observe(node);
    window.addEventListener("resize", resize);
    resize();
    return () => { observer.disconnect(); window.removeEventListener("resize", resize); cancelAnimationFrame(frame); };
  }, [api, settings, library, blank, state.active, error, state.notice, network.error]);

  return <main className="browser-app">
    <header className="browser-chrome">
      <div className="browser-tabbar" onMouseDown={e => {
        if (e.button !== 0 || (e.target as HTMLElement).closest("button,input,select,[role=tab]")) return;
        e.preventDefault(); action(api.window(e.detail === 2 ? "maximize" : "drag"));
      }}>
        <div className="browser-brand" title="Atsumi Browser" aria-label="Atsumi Browser"><img src={atsumiLogo} alt="Atsumi" draggable={false}/></div>
        <div className="browser-tabs" role="tablist" aria-label="브라우저 탭">
          {state.tabs.map(tab => <div className={"browser-tab " + (tab.id === state.active ? "active" : "")} key={tab.id}>
            <button type="button" role="tab" aria-selected={tab.id === state.active} title={tab.title + "\n" + tab.url} onClick={() => { setSettings(false); setLibrary(null); action(api.tab("select", tab.id)); }}>
              {tab.loading ? <span className="browser-spinner" aria-label="불러오는 중"/> : <FluentIcon glyph={tab.url === "about:blank" ? "\uE8A5" : "\uE774"}/>}
              <span>{tab.title || "새 탭"}</span>
            </button>
            <IconButton className="browser-tab-close" label={(tab.title || "새 탭") + " 닫기"} glyph="\uE711" onClick={() => action(api.tab("close", tab.id))}/>
          </div>)}
        </div>
        <IconButton className="browser-new-tab" label="새 탭" title="새 탭 · Ctrl+T" glyph="\uE710" onClick={() => newTab()}/>
        <div className="browser-drag-space" title="드래그하여 창 이동 · 두 번 클릭하여 최대화"/>
        <div className="browser-window-controls">
          <IconButton label="최소화" glyph="\uE921" onClick={() => action(api.window("minimize"))}/>
          <IconButton label={state.maximized ? "이전 크기로" : "최대화"} glyph={state.maximized ? "\uE923" : "\uE922"} onClick={() => action(api.window("maximize"))}/>
          <IconButton className="browser-window-close" label="브라우저 닫기" glyph="\uE8BB" onClick={() => action(api.window("close"))}/>
        </div>
      </div>
      <form className="browser-toolbar" onSubmit={e => { e.preventDefault(); go(address); }}>
        <div className="browser-navigation">
          <IconButton label="뒤로" title="뒤로 · Alt+←" glyph="\uE72B" disabled={blank || !current?.canGoBack} onClick={() => action(api.tab("back"))}/>
          <IconButton label="앞으로" title="앞으로 · Alt+→" glyph="\uE72A" disabled={blank || !current?.canGoForward} onClick={() => action(api.tab("forward"))}/>
          <IconButton label="새로고침" title="새로고침 · F5" glyph="\uE72C" disabled={blank} onClick={() => action(api.tab("reload"))}/>
        </div>
        <label className="browser-address"><FluentIcon glyph="\uE774"/>
          <input ref={input} aria-label="주소 또는 검색어" placeholder="주소 또는 검색어 입력" value={address} onChange={e => setAddress(e.target.value)} spellCheck={false} autoComplete="off"/>
          <IconButton className={bookmarked ? "browser-bookmark saved" : "browser-bookmark"} label={bookmarked ? "북마크 해제" : "북마크 추가"} title="북마크 · Ctrl+D" glyph={bookmarked ? "\uE735" : "\uE734"} disabled={blank} onClick={() => void toggleBookmark()}/>
        </label>
        <button type="button" className={"browser-connection " + network.phase} aria-label="브라우저 연결 설정" title={label} aria-expanded={settings} onClick={() => { setLibrary(null); setSettings(open => !open); }}>
          <span className="browser-connection-dot"/><span>{label}</span><FluentIcon glyph={settings ? "\uE70E" : "\uE70D"}/>
        </button>
        <IconButton label="방문 기록" title="방문 기록 · Ctrl+H" glyph="\uE81C" onClick={() => openLibrary("history")}/>
        <IconButton label="브라우저 설정" glyph="\uE713" onClick={() => openLibrary("preferences")}/>
      </form>
    </header>

    {(error || network.error) && <div role="alert" className="browser-alert"><FluentIcon glyph="\uE7BA"/><span>{error || network.error}</span>{error && <IconButton label="오류 닫기" glyph="\uE711" onClick={() => setError(null)}/>}</div>}
    {state.notice && <div role="status" className="browser-notice"><FluentIcon glyph="\uE896"/><span>{state.notice}{state.downloads > 0 ? " · " + state.downloads + "개" : ""}</span></div>}

    <div className="browser-content" ref={content}>
      {library ? <BrowserLibrary kind={library} api={api} state={state} onKind={setLibrary} onOpen={(url, fresh) => fresh ? newTab(url) : go(url)} onClose={() => setLibrary(null)} onChanged={() => setBookmarkRevision(n => n + 1)}/> : settings ? <section className={"browser-network-panel " + (showGuide ? "" : "compact")} aria-label="브라우저 연결 설정">
        <div className="browser-panel-heading">
          <div><span className="browser-eyebrow">ATSUMI BROWSER</span><h1>연결 설정</h1><p>이 브라우저에서 사용할 연결 방식을 선택하세요.</p></div>
          <IconButton label="연결 설정 닫기" title="브라우저로 돌아가기 · Esc" glyph="\uE711" onClick={() => setSettings(false)}/>
        </div>
        <div className={"browser-settings-grid " + (showGuide ? "with-guide" : "")}>
          <div className="browser-settings-primary">
            <section className={"browser-network-card " + network.phase}>
              <div className="browser-card-heading">
                <div className="browser-service-icon"><BrowserSymbol kind="shield"/></div>
                <div className="browser-card-title"><h2>Proton VPN</h2><span className="browser-muted">브라우저 전용</span></div>
                <NetworkSwitch label="Proton VPN" checked={network.enabled} disabled={networkLocked || !network.enabled && !network.profile} onClick={() => void updateNetwork(!network.enabled, network.fragment)}/>
              </div>
              <div className="browser-connection-summary" role="status">
                {connecting ? <span className="browser-spinner"/> : <span className="browser-connection-dot"/>}
                <strong>{label}</strong>
                <p>{connected ? "이 브라우저는 VPN을 통해 접속하고 있습니다." : connecting ? "서버에 연결하고 실제 접속 위치를 확인하고 있습니다." : network.enabled ? "연결이 확인될 때까지 이 브라우저의 통신을 멈춥니다." : network.profile ? "준비됐습니다. 위 스위치를 켜서 연결하세요." : "처음 연결하기 안내에 따라 설정 파일을 가져오세요."}</p>
              </div>
              {profiles.length > 0 && <div className="browser-country-picker">
                <label htmlFor="browser-country">국가 선택 <span>{profiles.length}개 저장됨</span></label>
                <select id="browser-country" aria-label="VPN 국가" value={network.profileId ?? ""} disabled={networkLocked} title={selectedProfile?.name}
                  onChange={e => void updateNetwork(network.enabled, network.fragment, e.target.value)}>
                  {!network.profileId && <option value="" disabled>국가를 선택하세요</option>}
                  {orderedProfiles.map(profile => <option key={profile.id} value={profile.id}>{profile.countryHint ? countryLabel(profile.countryHint) : "국가 미확인"} · {profile.name}</option>)}
                </select>
                <p>{network.enabled ? "선택한 국가로 바로 다시 연결합니다." : "VPN을 켜면 선택한 국가로 연결합니다."}</p>
              </div>}
              <dl className="browser-profile-details">
                <div><dt>설정 파일</dt><dd>{network.profile ?? "등록 전"}</dd></div>
                {connected && <><div><dt>접속 국가</dt><dd>{countryLabel(network.country)}</dd></div><div><dt>외부 IP</dt><dd>{network.ip ?? "확인 중"}</dd></div></>}
              </dl>
              <div className="browser-card-actions">
                <button type="button" className={"browser-button " + (network.profile ? "" : "primary")} disabled={networkLocked} onClick={() => void importConfig()}><FluentIcon glyph="\uE8B5"/>{network.profile ? "국가 추가" : "설정 파일 가져오기"}</button>
                {network.enabled && network.phase === "error" && <button type="button" className="browser-button primary" disabled={networkLocked} onClick={() => void updateNetwork(true, network.fragment)}><FluentIcon glyph="\uE72C"/>다시 연결</button>}
              </div>
              {network.profile && <p className="browser-card-hint">한 번 등록한 국가는 파일을 다시 받지 않고 바꿀 수 있습니다.</p>}
              {state.downloads > 0 && <p className="browser-card-hint" role="status">이 브라우저의 다운로드 {state.downloads}개가 끝나면 연결 설정을 바꿀 수 있습니다.</p>}
            </section>

            <section className="browser-network-card">
              <div className="browser-card-heading">
                <div className="browser-service-icon secondary"><BrowserSymbol kind="split"/></div>
                <div className="browser-card-title"><h2>SNI 파편화</h2><span className="browser-muted">선택 사항</span></div>
                <NetworkSwitch label="SNI 파편화" checked={network.fragment} disabled={networkLocked} onClick={() => void updateNetwork(network.enabled, !network.fragment)}/>
              </div>
              <p className="browser-card-description">일부 통신망에서 사이트 연결이 막힐 때 시도해 보세요. <strong>접속 국가와 IP는 바뀌지 않습니다.</strong></p>
            </section>
            <div className="browser-scope-note"><FluentIcon glyph="\uE946"/><p>기존 Atsumi 녹화·다운로드는 그대로 이어집니다. 연결 방식을 바꾸면 이 브라우저의 열린 페이지만 다시 불러옵니다.</p></div>
            {network.profile && <button type="button" className="browser-guide-toggle" aria-expanded={showGuide} onClick={() => setGuideExpanded(open => !open)}><FluentIcon glyph="\uE897"/>Proton 처음 연결하기<FluentIcon glyph={showGuide ? "\uE70E" : "\uE70D"}/></button>}
          </div>

          {showGuide && <aside className="browser-setup-guide" aria-label="Proton 처음 연결하기">
            <div className="browser-guide-heading"><h2>처음 연결하기</h2><span>원하는 국가별로 처음 한 번만 준비하세요</span></div>
            <SetupStep number={1} title="Proton에서 파일 받기">
              <p>다운로드 페이지를 아래로 내려 <strong>WireGuard 설정</strong>을 찾으세요. Windows 앱 설치 파일이나 OpenVPN 파일은 여기서 사용하지 않습니다.</p>
              <button type="button" className="browser-button" onClick={() => newTab("https://account.protonvpn.com/downloads")}>Proton 다운로드 열기<FluentIcon glyph="\uE8A7"/></button>
              <p className="browser-account-hint">계정이 없다면 <button type="button" className="browser-text-button" onClick={() => newTab("https://account.protonvpn.com/signup")}>무료 계정 만들기</button>부터 진행하세요.</p>
              <dl className="browser-setup-fields">
                <div><dt>설정 이름</dt><dd>Atsumi Browser JP <span>알아보기 쉬운 이름이면 됩니다.</span></dd></div>
                <div><dt>플랫폼</dt><dd>Windows</dd></div>
                <div><dt>VPN Accelerator</dt><dd><span className="browser-value-on">켜기</span></dd></div>
                <div><dt>Port Forwarding</dt><dd>끄기 <span>일반 웹 탐색에는 필요하지 않습니다.</span></dd></div>
                <div><dt>서버</dt><dd>무료(Free) → 원하는 국가 <span>일본은 JP-FREE, 미국은 US-FREE입니다.</span></dd></div>
              </dl>
              <p>국가 안에서는 <strong>부하(%)가 낮은 서버</strong>를 골라 <strong>생성 → 다운로드</strong>를 누르세요. <code>.conf</code>로 끝나는 파일이 저장됩니다.</p>
              <details className="browser-country-help"><summary>어느 국가를 고르면 되나요?</summary><p>이용하려는 사이트가 허용하는 국가를 고르세요. 가까운 일본 서버부터 시험할 수 있습니다. 무료로 제공되는 국가는 계정 화면에서 확인하며, 사이트에 따라 VPN 접속 자체를 제한할 수도 있습니다.</p></details>
            </SetupStep>
            <SetupStep number={2} title="Atsumi에 가져오기">
              <p><strong>설정 파일 가져오기</strong>에서 방금 받은 <code>.conf</code>를 선택하세요. 다른 국가도 쓰려면 해당 국가의 파일을 한 번 받아 <strong>국가 추가</strong>로 등록하세요. 기존 설정은 남아 있습니다.</p>
              <p className="browser-muted">설정 파일에는 연결키가 들어 있습니다. 공유하지 마세요. 앱은 Windows 계정으로 암호화해 보관합니다.</p>
            </SetupStep>
            <SetupStep number={3} title="VPN 켜고 위치 확인하기">
              <p><strong>Proton VPN</strong> 스위치를 켜세요. <strong>VPN 연결됨 · 국가</strong>와 외부 IP가 표시되면 준비가 끝납니다. 다음부터는 <strong>국가 선택</strong>에서 바로 바꾸고, 이 스위치로 켜고 끄면 됩니다. 키가 만료되거나 폐기된 경우에만 새 파일이 필요합니다.</p>
            </SetupStep>
          </aside>}
        </div>
      </section> : blank ? <section className="browser-home">
        <img className="browser-home-logo" src={atsumiLogo} alt=""/>
        <div className="browser-home-heading"><h1>Atsumi Browser</h1><p>새로운 페이지를 열어 보세요.</p></div>
        <form className="browser-home-search" onSubmit={e => { e.preventDefault(); go(address); }}>
          <FluentIcon glyph="\uE721"/><input aria-label="새 탭에서 주소 또는 검색어 입력" placeholder="주소 또는 검색어 입력" value={address} onChange={e => setAddress(e.target.value)} spellCheck={false}/>
          <button type="submit" className="browser-icon-button" aria-label="이동" disabled={!address.trim()}><FluentIcon glyph="\uE72A"/></button>
        </form>
        {state.recentlyClosed > 0 && <button type="button" className="browser-button browser-reopen" onClick={() => shortcut.current("restore")}><FluentIcon glyph="\uE81C"/>닫은 탭 다시 열기 <kbd>Ctrl+Shift+T</kbd></button>}
        <button type="button" className="browser-home-connection" aria-label="Proton VPN 연결 설정" onClick={() => setSettings(true)}>
          <BrowserSymbol kind="shield"/><span><strong>{label}</strong><span>{network.profile ? "연결 설정" : "Proton 무료 VPN 처음 연결하기"}</span></span><FluentIcon glyph="\uE76C"/>
        </button>
      </section> : null}
    </div>
  </main>;
}
