import { isolatedFrame } from './isolation.js';
const { emit } = isolatedFrame();
const [{ dt: react, ut: reactDOM }, { rt: Textarea }, { createSurfaces }, { fixture }] = await Promise.all([
  import('./assets/common-vendor-pcHQV1G2.js'), import('./assets/vendor-DQYi7KBW.js'), import('./original-surfaces.js'), import('./surface-data.js'),
]);
const K = react(), DOM = reactDOM(), { Chat } = createSurfaces(K, DOM, Textarea, fixture, message => emit('event', message));
const replay = new URLSearchParams(location.search).get('mode') === 'vod';
const root = DOM.createRoot(document.querySelector('#chat-root'));
root.render(K.createElement(Chat, { replay, onCollapse: () => emit('collapse-chat') }));
emit('ready');
emit('event', `${replay ? '녹화본 채팅 검색' : '원본 라이브 채팅 구조'} · 로컬 샘플 ${fixture.messages.length}개`);
window.addEventListener('pagehide', () => root.unmount());
