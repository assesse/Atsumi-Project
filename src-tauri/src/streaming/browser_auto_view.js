// Native-created background receivers only. No synthetic clicks on login,
// subscription, ad, extension-install or other access/consent dialogs.
(() => {
  if (location.origin !== "https://chzzk.naver.com" || window.__atsumiAutoReceiver) return;
  window.__atsumiAutoReceiver = true;
  const play = () => {
    const video = [...document.querySelectorAll("video")]
      .sort((a, b) => b.clientWidth * b.clientHeight - a.clientWidth * a.clientHeight)[0];
    if (!video) return;
    video.muted = true; // Original encoded audio is still saved, not re-recorded.
    if (video.paused && !video.ended) video.play().catch(() => {});
  };
  const timer = setInterval(play, 3000);
  window.addEventListener("pagehide", () => clearInterval(timer), { once: true });
})();
