(() => {
  const nonce = __ATSUMI_X_NONCE__;
  const empty = {nonce};
  if (window !== window.top || location.protocol !== 'https:' || !['x.com','www.x.com'].includes(location.hostname)
      || location.pathname === '/login' || location.pathname.startsWith('/i/flow/')) return empty;
  const profile = document.querySelector('a[data-testid="AppTabBar_Profile_Link"]');
  const account = document.querySelector('[data-testid="SideNav_AccountSwitcher_Button"]');
  let handle;
  try { const url = new URL(profile?.getAttribute('href'), location.origin); if (url.origin !== location.origin) return empty; handle = /^\/([A-Za-z0-9_]{1,15})\/?$/.exec(url.pathname)?.[1]?.toLowerCase(); } catch { return empty; }
  return handle && account ? {nonce, handle} : empty;
})()
