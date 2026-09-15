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
export function mountRecordingMetadata(player, initial) {
  const slot = player.querySelector('.header_info');
  if (!slot) throw new Error('Original recording header slot missing');
  const root = DOM.createRoot(slot);
  const update = value => root.render(K.createElement(BroadcastInfo, { ...savedMetadata(value), wide: true, replay: true }));
  update(initial);
  return { update, dispose: () => root.unmount() };
}
