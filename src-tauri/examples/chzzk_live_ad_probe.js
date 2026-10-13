// Read-only observation: no account, chat text, URL queries, or response bodies.
(() => {
  const visible = n => {
    if (!n) return false;
    const r = n.getBoundingClientRect(), s = getComputedStyle(n);
    return r.width > 0 && r.height > 0 && s.display !== 'none' && s.visibility !== 'hidden' && Number(s.opacity) > 0;
  };
  const adNodes = [...document.querySelectorAll('#preAdVideoContainer video,.pzp-pc--ad video,.pzp-pc--ad-break video,[aria-label="동영상 광고"]')];
  return {
    adPolicy: window.__atsumiVideoAdPolicy || null,
    adPlayEvents: window.__atsumiAdProbe?.adPlayEvents || 0,
    videos: [...document.querySelectorAll('video')].map(v => ({time:v.currentTime,ready:v.readyState,paused:v.paused,width:v.videoWidth,height:v.videoHeight,visible:visible(v),inAd:!!v.closest('#preAdPlayerWrapper,.pzp-pc--ad,.pzp-pc--ad-break'),mediaHost:(()=>{try{return new URL(v.currentSrc).hostname}catch{return ''}})()})),
    resourceRoutes:[...new Set(performance.getEntriesByType('resource').map(r=>{try{const u=new URL(r.name);if(u.hostname.endsWith('veta.naver.com')||u.hostname==='api.chzzk.naver.com'||/ad.*\.pstatic\.net$/.test(u.hostname))return u.hostname+(u.pathname.startsWith('/service/t/')?'/service/t/REDACTED':u.pathname.replace(/[a-f0-9]{32,}/g,'ID').replace(/\d{4,}/g,'N'));}catch{}return null;}).filter(Boolean))].slice(0,100),
    adPlaying: adNodes.some(visible),
    adText: [...document.querySelectorAll('button,.pzp-pc__ad-text,.pzp-pc__ad-skip-button')].some(n => visible(n) && /광고 페이지 보기|초 후 SKIP|광고 건너뛰기/.test(n.textContent || n.getAttribute('aria-label') || '')),
    adWarning: /광고 차단 프로그램을 사용 중이신가요/.test(document.body.innerText),
    controls: [...document.querySelectorAll('.pzp-pc,.pzp-pc__bottom,.pzp-pc__bottom-buttons-right,.pzp-pc__fullscreen-button,#atsumi-player-controls,#atsumi-player-top-controls')].map(n => ({id:n.id,classes:n.className,opacity:getComputedStyle(n).opacity,visibility:getComputedStyle(n).visibility,transition:getComputedStyle(n).transition})),
    livePlayer: !!document.querySelector('.pzp-pc video,.webplayer video'),
  };
})()
