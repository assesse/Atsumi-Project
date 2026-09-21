// Native-created background receivers only. No synthetic clicks on login,
// subscription, ad, extension-install or other access/consent dialogs.
(() => {
  if (location.origin !== "https://chzzk.naver.com" || window.__atsumiAutoReceiver) return;
  let viewing = false, multiview = false, revision = -1;
  const muteChoices = new WeakMap();
  const refresh = () => {
    window.__atsumiMultiView?.setVideoOnly(!viewing || multiview);
    window.__atsumiPlayerUI?.configure({ automaticWatch: !viewing, multiview });
  };
  Object.defineProperty(window, "__atsumiAutoReceiver", { value: Object.freeze({
    configure: (next) => {
      if (!Number.isSafeInteger(next?.revision) || next.revision < revision || typeof next.viewing !== "boolean") return;
      const changed = viewing !== next.viewing;
      const videos = changed ? [...document.querySelectorAll("video")] : [];
      // Save the user's choice BEFORE closing the background audio gate. Open
      // it BEFORE restoring that choice, or volumechange will immediately mute
      // the original player again. Neither a resize nor a gate grant sets volume.
      if (changed && viewing) {
        for (const video of videos) muteChoices.set(video, video.muted);
      }
      if (typeof next.audioEnabled === "boolean") {
        window.dispatchEvent(new CustomEvent("atsumi-multiview-audio", {
          detail: { enabled: next.viewing && next.audioEnabled, applyToMedia: false },
        }));
      }
      if (changed && next.viewing) {
        for (const video of videos) video.muted = muteChoices.get(video) ?? false;
      }
      revision = next.revision; viewing = next.viewing; multiview = next.multiview === true;
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
