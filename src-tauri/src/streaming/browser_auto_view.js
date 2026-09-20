// Native-created background receivers only. No synthetic clicks on login,
// subscription, ad, extension-install or other access/consent dialogs.
(() => {
  if (location.origin !== "https://chzzk.naver.com" || window.__atsumiAutoReceiver) return;
  let viewing = false, revision = -1;
  const muteChoices = new WeakMap();
  const refresh = () => {
    window.__atsumiMultiView?.setVideoOnly(!viewing);
    window.__atsumiPlayerUI?.configure({ automaticWatch: !viewing, multiview: false });
  };
  Object.defineProperty(window, "__atsumiAutoReceiver", { value: Object.freeze({
    configure: (next) => {
      if (!Number.isSafeInteger(next?.revision) || next.revision < revision || typeof next.viewing !== "boolean") return;
      if (viewing !== next.viewing) {
        for (const video of document.querySelectorAll("video")) {
          if (viewing) muteChoices.set(video, video.muted);
          else video.muted = muteChoices.get(video) ?? false;
        }
      }
      revision = next.revision; viewing = next.viewing;
      // Only native presentation calls change this flag; neither the capture
      // session nor the page/navigation is recreated when borrowing the view.
      refresh();
      if (!document.getElementById("atsumi-auto-watch-style")) {
        const style = document.createElement("style"); style.id = "atsumi-auto-watch-style";
        style.textContent = "#atsumi-mado-toggle{display:none!important}";
        (document.head || document.documentElement)?.appendChild(style);
      }
    },
    refresh,
  }) });
  const play = () => {
    const video = [...document.querySelectorAll("video")]
      .sort((a, b) => b.clientWidth * b.clientHeight - a.clientWidth * a.clientHeight)[0];
    if (!video || viewing) return; // Foreground pause/volume belong to the viewer.
    video.muted = true;
    if (video.paused && !video.ended) video.play().catch(() => {});
  };
  const timer = setInterval(play, 3000);
  window.addEventListener("pagehide", () => clearInterval(timer), { once: true });
})();
