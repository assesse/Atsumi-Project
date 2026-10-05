// Native-created background receivers only. Scheduled recording may select the
// official install-free quality option, never login/permissions/install/consent.
(() => {
  if (location.origin !== "https://chzzk.naver.com" || window.__atsumiAutoReceiver) return;
  let viewing = false, multiview = false, revision = -1;
  let preparingRecording = false, standardAttempted = false, preparation = "disabled";
  const muteChoices = new WeakMap();
  const refresh = () => {
    window.__atsumiMultiView?.setVideoOnly(!viewing || multiview);
    window.__atsumiPlayerUI?.configure({ automaticWatch: !viewing, multiview });
  };
  Object.defineProperty(window, "__atsumiAutoReceiver", { value: Object.freeze({
    prepareRecording: () => { preparingRecording = true; if (preparation === "disabled") preparation = "waiting_player"; },
    isPreparingRecording: () => preparingRecording,
    getPreparationStatus: () => preparation,
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
  const chooseStandardQuality = (video) => {
    if (!preparingRecording || viewing) return;
    const capture = window.__atsumiEncodedCapture?.getStatus?.();
    if (capture?.active || capture?.starting || capture?.stopping) return;
    // Do not downgrade an already usable receiver (grid or standard quality).
    if (video?.readyState >= 2 && video.videoWidth > 0) { preparation = "video_ready"; return; }
    if (standardAttempted) return;
    const choices = [...document.querySelectorAll('button,a,[role="button"],[role="link"]')].filter(element => {
      if ((element.textContent ?? "").replace(/\s+/g, " ").trim() !== "설치없이 일반 화질 시청" ||
          element.disabled || element.getAttribute("aria-disabled") === "true" || element.getClientRects().length === 0) return false;
      const css = window.getComputedStyle(element);
      if (css.display === "none" || css.visibility === "hidden") return false;
      // This must be an in-page quality choice, not a download/navigation link.
      const href = element.getAttribute("href");
      if (href && !href.startsWith("#")) return false;
      return true;
    });
    if (choices.length !== 1) {
      preparation = choices.length > 1 ? "standard_quality_ambiguous" : "waiting_player";
      return;
    }
    standardAttempted = true; // At most one attempt per document; no retry-click loop.
    window.__atsumiQuality?.useStandardQuality?.();
    try { choices[0].click(); preparation = "standard_quality_selected"; }
    catch { preparation = "standard_quality_failed"; }
  };
  const play = () => {
    const video = [...document.querySelectorAll("video")]
      .sort((a, b) => b.clientWidth * b.clientHeight - a.clientWidth * a.clientHeight)[0];
    chooseStandardQuality(video);
    if (!video || viewing) return; // Foreground pause/volume belong to the viewer.
    video.muted = true;
    if (video.paused && !video.ended) video.play().catch(() => {});
  };
  const timer = setInterval(play, 3000);
  window.addEventListener("pagehide", () => clearInterval(timer), { once: true });
})();
