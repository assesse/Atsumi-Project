// Local fixture only: use production behavior with a synthetic location
// binding. There is no actual CHZZK page, media, account or network.
import bridge from "../src-tauri/src/streaming/browser_multiview.js?raw";
import "../src/features/streaming/generated/accepted/original.css";
import "../src/features/streaming/generated/accepted/fonts.css";

const id = "a".repeat(32), root = document.getElementById("root")!;
root.innerHTML = `<div style="display:flex;gap:12px;margin-bottom:20px"><button data-width="480">480px</button><button data-width="320">320px</button><button data-width="220">220px</button></div>
  <section id="chat-fixture" class="theme_dark" style="width:480px;height:260px;background:#101213;border:1px solid #333;border-radius:6px;overflow:hidden;--color-content-02:var(--sem-color-content-neutral-primary,#dfe2ea);--color-content-04:var(--sem-color-content-neutral-cool-stronger,#dfe2ea)">
  <div class="_container_1e2su_2"><h2 class="_title_1e2su_12" style="margin:0">채팅</h2>
  <div class="_wrapper_1e2su_28 _scale_1e2su_39"><button class="_scale_button_1e2su_55" aria-label="채팅 크기 마이너스">−</button><span class="_scale_text_1e2su_46">100%</span><button class="_scale_button_1e2su_55" aria-label="채팅 크기 플러스">+</button></div>
  <div class="_wrapper_1e2su_28 _menu_1e2su_36"><button class="_button_1e2su_69" aria-label="더보기" style="font-size:20px;line-height:24px">⋮</button></div></div>
  <p style="padding:12px;color:#9ba1a6;font:13px system-ui">로컬 채팅 영역</p></section>`;
const script = document.createElement("script");
script.textContent = `(()=>{const testLocation=new URL('https://chzzk.naver.com/live/${id}/chat');${bridge.replaceAll("window.location", "testLocation")}})();
  window.__atsumiMultiView.configureChat({channelId:'${id}',number:2,channelName:'로션욤'});`;
document.body.appendChild(script);
root.querySelectorAll<HTMLButtonElement>("button[data-width]").forEach(button => button.addEventListener("click", () => {
  document.getElementById("chat-fixture")!.style.width = `${button.dataset.width}px`;
  window.dispatchEvent(new Event("resize"));
}));
void document.fonts.ready.then(() => window.dispatchEvent(new Event("resize")));
