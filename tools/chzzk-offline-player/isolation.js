export function isolatedFrame() {
  const channel = 'chzzk-offline-shell-proof', nonce = location.hash.slice(1);
  if (window === window.top || window.origin !== 'null' || !/^[a-f0-9-]{36}$/.test(nonce)) throw new Error('Isolated local frame required');
  const parentOrigin = new URL(location.href).origin;
  const emit = (type, data) => parent.postMessage({ channel, nonce, type, data }, '*');
  document.addEventListener('securitypolicyviolation', event => emit('event', `외부 연결 차단: ${event.effectiveDirective} · ${event.blockedURI}`));
  window.addEventListener('error', event => emit('error', event.message || '리소스 오류'));
  window.addEventListener('unhandledrejection', event => emit(event.reason?.name === 'AjaxError' ? 'event' : 'error', String(event.reason)));
  function memoryStorage() {
    const values = new Map();
    return { get length() { return values.size; }, key: i => [...values.keys()][i] ?? null, getItem: key => values.get(String(key)) ?? null,
      setItem(key, value) { key=String(key);value=String(value);if(key.length<256&&value.length<65536&&(values.has(key)||values.size<100))values.set(key,value); }, removeItem: key => values.delete(String(key)), clear: () => values.clear() };
  }
  for (const key of ['localStorage', 'sessionStorage']) Object.defineProperty(window, key, { value: memoryStorage() });
  const accepts = event => event.source === parent && event.origin === parentOrigin && event.data?.channel === channel && event.data.nonce === nonce;
  window.addEventListener('message', event => {
    if (accepts(event) && event.data.type === 'test-egress') fetch('https://example.invalid/chzzk-offline-proof').catch(() => emit('event', '외부 연결 차단 시험 완료'));
  });
  return { emit, accepts };
}
