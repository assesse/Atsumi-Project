// Saved text and a bounded, native-validated inline raster only. No remote URL.
import { dt as react, ut as reactDOM } from './common-vendor-pcHQV1G2.js';
import { createSurfaces } from './original-surfaces.js';
const K = react(), DOM = reactDOM();
const fixture = { title: '', channelName: '', viewers: 0, uptime: '', profileImage: new URL('./assets/default_profile_dark.png', import.meta.url).href };
const { BroadcastInfo } = createSurfaces(K, DOM, null, fixture, () => {});
export function savedMetadata(value) {
  const text = (value, limit) => typeof value === 'string' ? value.slice(0, limit).trim() : '';
  return {
    title: text(value?.title, 2048), channelName: text(value?.channelName, 160),
    recordedAt: Number.isFinite(value?.recordedAt) && value.recordedAt > 0 && value.recordedAt <= 4102444800000 ? value.recordedAt : null,
    profileImage: typeof value?.profileImage === 'string' && value.profileImage.length <= 1_500_000 && /^data:image\/(?:png|jpeg|gif|webp);base64,[a-zA-Z0-9+/]+={0,2}$/.test(value.profileImage) ? value.profileImage : fixture.profileImage,
  };
}
export function mountRecordingMetadata(player, initial, onChannel = () => {}) {
  const slot = player.querySelector('.header_info');
  if (!slot) throw new Error('Original recording header slot missing');
  const releaseProfile = bindRecordedProfile(slot, onChannel);
  const root = DOM.createRoot(slot);
  const update = value => root.render(K.createElement(BroadcastInfo, { ...savedMetadata(value), wide: true, replay: true }));
  update(initial);
  return { update, dispose: () => { releaseProfile(); root.unmount(); } };
}

// The opaque player may request only "open this recording's channel", never a
// URL. Retain the original header layout and make its saved avatar accessible.
export function bindRecordedProfile(slot, onChannel) {
  const selector = 'img[width="60"][height="60"]';
  const decorate = () => slot.querySelectorAll(selector).forEach(image => {
    image.tabIndex = 0; image.setAttribute('role', 'button');
    image.setAttribute('aria-label', '녹화 채널 페이지 열기');
    image.title = '녹화 채널 페이지 열기'; image.style.cursor = 'pointer';
  });
  const activate = event => {
    if (!(event.target instanceof Element) || !event.target.matches(selector)) return;
    if (event.type === 'keydown' && event.key !== 'Enter' && event.key !== ' ') return;
    event.preventDefault(); event.stopPropagation(); onChannel();
  };
  const observer = new MutationObserver(decorate);
  observer.observe(slot, { childList: true, subtree: true }); decorate();
  slot.addEventListener('click', activate); slot.addEventListener('keydown', activate);
  return () => { observer.disconnect(); slot.removeEventListener('click', activate); slot.removeEventListener('keydown', activate); };
}
