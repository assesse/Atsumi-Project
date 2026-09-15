// Original SDK and controls; local media, host actions and explicit UI bindings.
const channel = 'chzzk-offline-shell-proof', nonce = location.hash.slice(1);
if (window === window.top || window.origin !== 'null' || !/^[a-f0-9-]{36}$/.test(nonce)) throw new Error('Isolated local frame required');
const parentOrigin = new URL(location.href).origin;
const emit = (type, data) => parent.postMessage({ channel, nonce, type, data }, '*');
document.addEventListener('securitypolicyviolation', event => emit('event', `외부 연결 차단: ${event.effectiveDirective} · ${event.blockedURI}`));
window.addEventListener('error', event => emit('error', event.message || '리소스 오류'));
window.addEventListener('unhandledrejection', event => {
  // The unchanged SDK rejects its analytics AJAX when CSP blocks Nelo. Keep
  // that evidence, but only a media/runtime error should mark playback failed.
  emit(event.reason?.name === 'AjaxError' ? 'event' : 'error', String(event.reason));
});
function memoryStorage() { const values = new Map(); return { get length() { return values.size; }, key: i => [...values.keys()][i] ?? null, getItem: key => values.get(String(key)) ?? null, setItem(key, value) { key = String(key); value = String(value); if (key.length < 256 && value.length < 65536 && (values.has(key) || values.size < 100)) values.set(key, value); }, removeItem: key => values.delete(String(key)), clear: () => values.clear() }; }
for (const key of ['localStorage', 'sessionStorage']) Object.defineProperty(window, key, { value: memoryStorage() });
const { S: loadPlayer } = await import('./assets/player-vendor-BYg0wCyN.js');
const P = loadPlayer();
const player = P.default.upgrade(document.querySelector('pzp-pc-layout'));
player.language = 'ko';
player.querySelector('pzp-pc-layout').sizeType = 'large';
player.querySelector('pzp-pc-setting-playbackrate-pane').playbackRates = [.25, .5, .75, 1, 1.25, 1.5, 1.75, 2];
const live = new URL(location.href).searchParams.get('mode') === 'live';
if (live) {
  const originalBadge = document.querySelector('#live-badge-template'), slot = player.querySelector('.live_time');
  if (slot && originalBadge) slot.append(originalBadge.content.cloneNode(true));
  // Exact original LIVE application's no-preroll / no-poster state bindings
  // (pinned index, Ue and Ke callbacks), not a replacement CSS/control layer.
  const dimmed = player.querySelector('.dimmed'), poster = player.querySelector('.pzp-pc__poster');
  if (dimmed) dimmed.style.display = 'none';
  if (poster) poster.style.display = 'none';
}
const state = () => {
  const video = document.querySelector('video');
  emit('state', { time: player.currentTime || 0, paused: player.paused, rate: player.playbackRate, width: video?.videoWidth || 0, height: video?.videoHeight || 0 });
};
for (const name of ['loadedmetadata', 'playing', 'pause', 'seeked', 'ratechange', 'volumechange', 'ended']) player.addEventListener(name, () => { state(); emit('event', `${name} · ${Number(player.currentTime || 0).toFixed(1)}초 · ${player.playbackRate}배속`); });
player.addEventListener('error', () => emit('error', '원본 플레이어의 로컬 영상 재생 오류'));
player.addEventListener('loadedmetadata', () => { const video = document.querySelector('video'); if (video) video.loop = true; emit('ready'); });
const wide = player.querySelector('pzp-pc-viewmode-button');
let infoRoot, chatHidden = false, renderInfo = () => {};
if (wide?.addEventListener) wide.addEventListener('change', () => { emit('wide', Boolean(wide.checked)); renderInfo(); });
document.addEventListener('fullscreenchange', () => renderInfo());
{
  const [{ dt: react, ut: reactDOM }, { rt: Textarea }, { createSurfaces }, { fixture }] = await Promise.all([
    import('./assets/common-vendor-pcHQV1G2.js'), import('./assets/vendor-DQYi7KBW.js'), import('./original-surfaces.js'), import('./surface-data.js'),
  ]);
  const K = react(), DOM = reactDOM(), { BroadcastInfo } = createSurfaces(K, DOM, Textarea, fixture, message => emit('event', message));
  const slot = player.querySelector('.header_info');
  if (!slot) throw new Error('Original broadcast header portal is missing');
  infoRoot = DOM.createRoot(slot);
  // This isolated player has no channel-info area below it. Use the original
  // detailed header in both widths; the SDK still owns hover/fade visibility.
  renderInfo = () => infoRoot.render(K.createElement(BroadcastInfo, { wide: true, chatHidden, replay: !live, recordedAt: live ? null : 1789272000000, onChat: () => emit('show-chat') }));
  renderInfo();
}
player.shadowRoot.addEventListener('click', event => {
  if (event.target.closest?.('.custom__shop-button,.custom__clip-button,.setting-radio-mode,.show_high_quality')) emit('event', '이 원본 버튼의 온라인 서비스는 연결하지 않았습니다.');
}, true);
player.muted = true;
player.srcObject = new P.DataProvider({ videoTracks: [{ src: new URL('/sample.mp4', location.href).href, id: 'local-original', label: '원본', selected: true }], textTracks: [] });
window.addEventListener('message', event => {
  if (event.source !== parent || event.origin !== parentOrigin || event.data?.channel !== channel || event.data.nonce !== nonce) return;
  if (event.data.type === 'chat-hidden' && typeof event.data.data === 'boolean') { chatHidden = event.data.data; renderInfo(); }
  if (event.data.type === 'test-egress') fetch('https://example.invalid/chzzk-offline-proof').catch(() => emit('event', '외부 연결 차단 시험 완료 · 원본 코드는 계속 로컬에서 실행'));
});
window.addEventListener('pagehide', () => { try { infoRoot?.unmount(); player.pause(); player.srcObject = null; } catch {} });
emit('event', `원본 ${live ? 'LIVE' : 'VOD'} HTML 표현식 + 원본 SDK 로컬 실행`);
