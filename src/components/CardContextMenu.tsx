import { useCallback, useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent, type MouseEvent } from "react";
import { createPortal } from "react-dom";
import "./CardContextMenu.css";

export type CardMenuItem = {
  id: string;
  label: string;
  shortcut?: string;
  disabled?: boolean;
  danger?: boolean;
  separator?: boolean;
  keepOpen?: boolean;
  action(): void;
};
type Anchor = { owner: HTMLElement; x: number; y: number };

// Decorative card content belongs to the card. Only real controls own their
// hitboxes; SVG paths inside a button must behave exactly like the button.
export function isCardControl(target: EventTarget | null) {
  return target instanceof Element && Boolean(target.closest('button, a, input, select, textarea, [role="button"], [contenteditable="true"]'));
}

export function useCardContextMenu() {
  const [anchor, setAnchor] = useState<Anchor | null>(null);
  const close = useCallback((restoreFocus = false) => {
    if (restoreFocus && anchor?.owner.isConnected) anchor.owner.focus({ preventScroll: true });
    setAnchor(null);
  }, [anchor]);
  const open = (event: MouseEvent<HTMLElement> | KeyboardEvent<HTMLElement>) => {
    if (event.defaultPrevented) return;
    event.preventDefault();
    event.stopPropagation();
    const owner = event.currentTarget;
    owner.focus({ preventScroll: true });
    const rect = owner.getBoundingClientRect();
    const pointer = "clientX" in event && Number.isFinite(event.clientX) && Number.isFinite(event.clientY) && (event.clientX !== 0 || event.clientY !== 0);
    // Opening another card's menu closes the old one without stealing focus.
    window.dispatchEvent(new Event("atsumi:card-menu-open"));
    setAnchor({ owner, x: pointer ? event.clientX : rect.left + Math.min(32, rect.width / 2), y: pointer ? event.clientY : rect.top + Math.min(32, rect.height / 2) });
  };
  const onKeyDown = (event: KeyboardEvent<HTMLElement>) => {
    if (event.target !== event.currentTarget || !(event.key === "ContextMenu" || (event.shiftKey && event.key === "F10"))) return false;
    open(event);
    return true;
  };
  return { anchor, close, open, onKeyDown };
}

export function CardContextMenu({ anchor, close, label, items }: {
  anchor: Anchor | null; close(restoreFocus?: boolean): void; label: string; items: CardMenuItem[];
}) {
  return anchor ? <OpenCardMenu key={`${anchor.x}:${anchor.y}`} anchor={anchor} close={close} label={label} items={items} /> : null;
}

function OpenCardMenu({ anchor, close, label, items }: {
  anchor: Anchor; close(restoreFocus?: boolean): void; label: string; items: CardMenuItem[];
}) {
  const menu = useRef<HTMLDivElement>(null);
  const [position, setPosition] = useState({ left: anchor.x, top: anchor.y });
  useLayoutEffect(() => {
    const node = menu.current;
    if (!node) return;
    const rect = node.getBoundingClientRect();
    setPosition({ left: Math.max(8, Math.min(anchor.x, window.innerWidth - rect.width - 8)), top: Math.max(8, Math.min(anchor.y, window.innerHeight - rect.height - 8)) });
    if (document.activeElement === anchor.owner || document.activeElement === document.body) {
      node.querySelector<HTMLButtonElement>("button:not(:disabled)")?.focus({ preventScroll: true });
    }
  }, [anchor, items]);
  useEffect(() => {
    const outside = (event: Event) => { if (!(event.target instanceof Node) || !menu.current?.contains(event.target)) close(); };
    const dismiss = () => close();
    const keys = (event: globalThis.KeyboardEvent) => {
      // A menu owns keyboard input, including the gallery's global shortcuts.
      event.stopPropagation();
      if (event.key === "Escape" || event.key === "Tab") { event.preventDefault(); close(true); return; }
      const buttons = [...(menu.current?.querySelectorAll<HTMLButtonElement>("button:not(:disabled)") ?? [])];
      const index = buttons.indexOf(document.activeElement as HTMLButtonElement);
      let next: number | undefined;
      if (event.key === "ArrowDown") next = (index + 1) % buttons.length;
      if (event.key === "ArrowUp") next = (index - 1 + buttons.length) % buttons.length;
      if (event.key === "Home") next = 0;
      if (event.key === "End") next = buttons.length - 1;
      if (next !== undefined) { event.preventDefault(); buttons[next]?.focus(); }
      if (event.key === "Enter" || event.key === " ") { event.preventDefault(); buttons[index]?.click(); }
    };
    document.addEventListener("pointerdown", outside, true);
    document.addEventListener("contextmenu", outside, true);
    document.addEventListener("scroll", outside, true);
    document.addEventListener("keydown", keys, true);
    window.addEventListener("resize", dismiss);
    window.addEventListener("blur", dismiss);
    window.addEventListener("atsumi:card-menu-open", dismiss);
    return () => {
      document.removeEventListener("pointerdown", outside, true);
      document.removeEventListener("contextmenu", outside, true);
      document.removeEventListener("scroll", outside, true);
      document.removeEventListener("keydown", keys, true);
      window.removeEventListener("resize", dismiss);
      window.removeEventListener("blur", dismiss);
      window.removeEventListener("atsumi:card-menu-open", dismiss);
    };
  }, [close]);
  return createPortal(<div ref={menu} role="menu" aria-label={label} className="card-context-menu" data-card-context-menu data-gallery-shortcuts-suspended style={position}
    onClick={(event) => event.stopPropagation()} onDoubleClick={(event) => event.stopPropagation()} onContextMenu={(event) => { event.preventDefault(); event.stopPropagation(); }}>
    <div className="card-context-menu-title" title={label}>{label}</div>
    {items.map((item) => <button type="button" key={item.id} role="menuitem" tabIndex={-1} disabled={item.disabled}
      className={`${item.danger ? "is-danger" : ""}${item.separator ? " has-separator" : ""}`}
      onClick={() => { if (!item.keepOpen) close(true); item.action(); }}><span>{item.label}</span>{item.shortcut ? <kbd>{item.shortcut}</kbd> : null}</button>)}
  </div>, anchor.owner.closest("dialog[open]") ?? document.body);
}
