const frame = document.querySelector('#player'), chat = document.querySelector('#chat'), status = document.querySelector('#status');
const events = [], channel = 'chzzk-offline-shell-proof';
let mode = 'live', nonce = '', chatNonce = '', chatHidden = false;
// Round only the empty host's video box to CSS pixels; original UI is untouched.
const videoHost = document.querySelector('#video-host');
const geometry = new ResizeObserver(([entry]) => {
  const height = `${Math.round(entry.contentRect.width * 9 / 16)}px`;
  if (videoHost.style.height !== height) videoHost.style.height = height;
});
geometry.observe(videoHost);
window.addEventListener('pagehide', () => geometry.disconnect());
function log(message) { events.push(message); document.querySelector('#events').textContent = events.slice(-24).join('\n'); }
function open() {
  nonce = crypto.randomUUID(); status.textContent = '원본 로딩 중…';
  document.body.classList.remove('wide');
  for (const value of ['live', 'vod']) document.getElementById(value).setAttribute('aria-pressed', String(value === mode));
  frame.src = `/frame.html?mode=${mode}#${nonce}`;
  chatNonce = crypto.randomUUID();
  chat.title = mode === 'vod' ? 'CHZZK 녹화본 채팅 검색' : 'CHZZK 원본 라이브 채팅';
  document.querySelector('#chat-status').textContent = '채팅 로딩 중…';
  chat.src = `/chat.html?mode=${mode}#${chatNonce}`;
}
function showChat(hidden) {
  chatHidden = hidden;
  chat.hidden = hidden;
  document.body.classList.toggle('chat-hidden', hidden);
  document.querySelector('#chat-toggle').setAttribute('aria-pressed', String(!hidden));
  frame.contentWindow.postMessage({ channel, nonce, type: 'chat-hidden', data: hidden }, '*');
}
document.querySelector('#chat-toggle').addEventListener('click', () => showChat(!chatHidden));
for (const value of ['live', 'vod']) document.getElementById(value).addEventListener('click', () => { mode = value; open(); });
document.querySelector('#reopen').addEventListener('click', open);
document.querySelector('#network-test').addEventListener('click', () => {
  frame.contentWindow.postMessage({ channel, nonce, type: 'test-egress' }, '*');
  chat.contentWindow.postMessage({ channel, nonce: chatNonce, type: 'test-egress' }, '*');
});
window.addEventListener('message', event => {
  const message = event.data;
  if (event.origin !== 'null' || message?.channel !== channel) return;
  if (event.source === chat.contentWindow && message.nonce === chatNonce) {
    if (message.type === 'event') log(`채팅 · ${message.data}`);
    if (message.type === 'ready') document.querySelector('#chat-status').textContent = '채팅 · 로컬 원본 준비됨';
    if (message.type === 'collapse-chat') showChat(true);
    if (message.type === 'error') { document.querySelector('#chat-status').textContent = '채팅 오류 · 검증 기록 확인'; log(message.data); }
    return;
  }
  if (event.source !== frame.contentWindow || message.nonce !== nonce) return;
  if (message.type === 'ready') { status.textContent = '로컬 원본 준비됨'; showChat(chatHidden); }
  if (message.type === 'event') log(message.data);
  if (message.type === 'state') {
    const data = message.data;
    status.textContent = `${data.paused ? '일시정지' : '재생 중'} · ${data.width}×${data.height} · ${data.rate}배속 · 로컬 파일`;
  }
  if (message.type === 'wide') document.body.classList.toggle('wide', message.data);
  if (message.type === 'show-chat') showChat(false);
  if (message.type === 'error') { status.textContent = '실행 오류 · 검증 기록 확인'; log(message.data); }
});
const manifest = await fetch('/manifest.json', { cache: 'no-store' }).then(response => response.json());
document.querySelector('#proof').textContent = `${manifest.originalFiles.length}개 원본 JS/CSS + ${manifest.resourceFiles.length}개 원본 글꼴·라이브러리·이미지 · SHA-256 확인\n외부 서버 연결: 두 프레임 모두 CSP로 차단\n원본 플레이어·라이브 채팅 유지 · 녹화본 입력/도구 영역만 채팅 검색으로 변경\n방송 정보·채팅: 원본 React 표현식 ${manifest.surfaces.records.length}개 · 로컬 샘플 데이터\n동영상 소스: sample.mp4 (로컬 합성 영상)\n\n${manifest.originalFiles.map(file => `${file.file}\n${file.sha256}`).join('\n')}`;
open();
