// Runs only inside the dedicated official account-status document. The native result is
// ONLY an enum: no profile fields, cookies or account identifiers are returned.
(async () => {
  if (location.href !== "https://comm-api.game.naver.com/nng_main/v1/user/getUserStatus" || window.top !== window) return "unknown";
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), 4000);
  try {
    const response = await fetch("https://comm-api.game.naver.com/nng_main/v1/user/getUserStatus", {
      method: "GET", credentials: "include", cache: "no-store", redirect: "error", signal: controller.signal,
    });
    if (!response.ok) return "unknown";
    const status = await response.json();
    if (status?.code !== 200 || typeof status?.content?.loggedIn !== "boolean") return "unknown";
    return status.content.loggedIn ? "signed_in" : "signed_out";
  } catch { return "unknown"; }
  finally { clearTimeout(timer); }
})()
