(message, right, bottom) => {
  if (location.origin !== "https://chzzk.naver.com" || !document.body || typeof message !== "string") return;
  document.getElementById("atsumi-recording-notice")?.remove();
  const notice = document.createElement("div");
  notice.id = "atsumi-recording-notice";
  notice.setAttribute("role", "status");
  notice.textContent = message;
  notice.style.cssText = `position:fixed;right:${right}px;bottom:${bottom}px;z-index:2147483647;pointer-events:none;max-width:calc(100vw - 40px);padding:11px 15px;border:1px solid #4c4852;border-radius:9px;background:#242127f5;color:#f2f0f4;font:13px/1.5 system-ui,sans-serif;box-shadow:0 4px 18px #0005`;
  document.body.appendChild(notice);
  setTimeout(() => notice.remove(), 2600);
}
