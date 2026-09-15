import { useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import "./generated/accepted/fonts.css";
import originalStyles from "./generated/accepted/original.css?raw";
import searchStyles from "./generated/accepted/replay-search.css?raw";
import adapterStyles from "./OriginalChzzkChatSurface.css?raw";

// A document's :root does not match inside a ShadowRoot. Keep the original
// tokens/values intact and scope their five declarations to this host instead.
export const shadowChatStyles = originalStyles.replaceAll(":root{", ":host{");

/** Style isolation only: no remote document, original app bootstrap, or account session. */
export function OriginalChzzkChatSurface({ children }: { children: (surface: ShadowRoot) => ReactNode }) {
  const host = useRef<HTMLElement>(null);
  const [surface, setSurface] = useState<ShadowRoot | null>(null);
  useLayoutEffect(() => {
    const element = host.current!;
    setSurface(element.shadowRoot ?? element.attachShadow({ mode: "open" }));
  }, []);
  return <aside ref={host} className="recording-replay-chat theme_dark" aria-label="저장 채팅 다시보기">
    {surface && createPortal(<>
      <style>{shadowChatStyles}</style>
      <style>{searchStyles}</style>
      <style>{adapterStyles}</style>
      <div className="original-chat-root theme_dark" data-theme="dark">{children(surface)}</div>
    </>, surface)}
  </aside>;
}
