import {
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type CSSProperties,
  type KeyboardEvent,
} from "react";
import { createPortal } from "react-dom";
import { FluentIcon } from "./FluentIcon";

export type DropdownOption<T extends string> = {
  value: T;
  label: string;
};

type DropdownSelectProps<T extends string> = {
  ariaLabel: string;
  value: T;
  options: readonly DropdownOption<T>[];
  onChange: (value: T) => void;
  prefix?: string;
  className?: string;
  variant?: "toolbar" | "field";
};

type MenuPlacement = {
  left: number;
  width: number;
  maxHeight: number;
  top?: number;
  bottom?: number;
};

const clamp = (value: number, minimum: number, maximum: number): number => (
  Math.min(Math.max(value, minimum), Math.max(minimum, maximum))
);

export function DropdownSelect<T extends string>({
  ariaLabel,
  value,
  options,
  onChange,
  prefix,
  className = "",
  variant = "field",
}: DropdownSelectProps<T>) {
  const id = useId();
  const trigger = useRef<HTMLButtonElement>(null);
  const menu = useRef<HTMLDivElement>(null);
  const optionButtons = useRef<Array<HTMLButtonElement | null>>([]);
  const [open, setOpen] = useState(false);
  const selectedIndex = Math.max(0, options.findIndex((option) => option.value === value));
  const [activeIndex, setActiveIndex] = useState(selectedIndex);
  const [placement, setPlacement] = useState<MenuPlacement>({ left: 8, width: 220, maxHeight: 320, top: 8 });
  const selected = options[selectedIndex] ?? options[0];
  const listboxId = `atsumi-dropdown-${id}`;
  const portalHost = trigger.current?.closest("dialog") ?? document.body;

  const updatePlacement = useCallback(() => {
    const node = trigger.current;
    if (!node) return;
    const rect = node.getBoundingClientRect();
    const viewportPadding = 8;
    const gap = 6;
    const preferredWidth = Math.max(rect.width, 220);
    const width = Math.min(preferredWidth, Math.max(180, window.innerWidth - viewportPadding * 2));
    const left = clamp(rect.left, viewportPadding, window.innerWidth - width - viewportPadding);
    const availableBelow = window.innerHeight - rect.bottom - viewportPadding - gap;
    const availableAbove = rect.top - viewportPadding - gap;
    const opensUp = availableBelow < 180 && availableAbove > availableBelow;
    const available = opensUp ? availableAbove : availableBelow;
    const maxHeight = Math.max(110, Math.min(320, available));
    setPlacement(opensUp
      ? { left, width, maxHeight, bottom: window.innerHeight - rect.top + gap }
      : { left, width, maxHeight, top: rect.bottom + gap });
  }, []);

  const close = useCallback((restoreFocus = false) => {
    setOpen(false);
    if (restoreFocus) window.requestAnimationFrame(() => trigger.current?.focus());
  }, []);

  const openAt = useCallback((index: number) => {
    if (!options.length) return;
    setActiveIndex(clamp(index, 0, options.length - 1));
    setOpen(true);
  }, [options.length]);

  useLayoutEffect(() => {
    if (!open) return;
    updatePlacement();
    const reposition = () => updatePlacement();
    window.addEventListener("resize", reposition);
    window.addEventListener("scroll", reposition, true);
    return () => {
      window.removeEventListener("resize", reposition);
      window.removeEventListener("scroll", reposition, true);
    };
  }, [open, updatePlacement]);

  useEffect(() => {
    if (!open) return;
    const pointerDown = (event: PointerEvent) => {
      const target = event.target as Node;
      if (!trigger.current?.contains(target) && !menu.current?.contains(target)) close();
    };
    document.addEventListener("pointerdown", pointerDown, true);
    return () => document.removeEventListener("pointerdown", pointerDown, true);
  }, [close, open]);

  useEffect(() => {
    if (!open) return;
    window.requestAnimationFrame(() => optionButtons.current[activeIndex]?.focus());
  }, [activeIndex, open]);

  const selectOption = (index: number) => {
    const option = options[index];
    if (!option) return;
    onChange(option.value);
    close(true);
  };

  const moveOptionFocus = (event: KeyboardEvent, index: number) => {
    event.preventDefault();
    setActiveIndex(clamp(index, 0, options.length - 1));
  };

  return (
    <div className={`atsumi-dropdown is-${variant}${open ? " is-open" : ""}${className ? ` ${className}` : ""}`}>
      <button
        ref={trigger}
        type="button"
        className="atsumi-dropdown-trigger"
        aria-label={ariaLabel}
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-controls={open ? listboxId : undefined}
        onClick={() => open ? close() : openAt(selectedIndex)}
        onKeyDown={(event) => {
          if (event.key === "ArrowDown") {
            event.preventDefault();
            openAt(Math.min(options.length - 1, selectedIndex + 1));
          } else if (event.key === "ArrowUp") {
            event.preventDefault();
            openAt(Math.max(0, selectedIndex - 1));
          } else if (event.key === "Home") {
            event.preventDefault();
            openAt(0);
          } else if (event.key === "End") {
            event.preventDefault();
            openAt(options.length - 1);
          } else if (event.key === "Escape" && open) {
            event.preventDefault();
            close();
          }
        }}
      >
        {prefix ? <span className="atsumi-dropdown-prefix">{prefix}</span> : null}
        <span className="atsumi-dropdown-value">{selected?.label ?? value}</span>
        <FluentIcon glyph="\uE70D" />
      </button>
      {open ? createPortal(
        <div
          ref={menu}
          id={listboxId}
          className="atsumi-dropdown-menu"
          role="listbox"
          aria-label={ariaLabel}
          style={{
            left: placement.left,
            width: placement.width,
            maxHeight: placement.maxHeight,
            ...(placement.top === undefined ? {} : { top: placement.top }),
            ...(placement.bottom === undefined ? {} : { bottom: placement.bottom }),
          } as CSSProperties}
        >
          {options.map((option, index) => (
            <button
              key={option.value}
              ref={(node) => { optionButtons.current[index] = node; }}
              type="button"
              role="option"
              aria-selected={option.value === value}
              className={index === activeIndex ? "is-active" : undefined}
              onPointerMove={() => setActiveIndex(index)}
              onClick={() => selectOption(index)}
              onKeyDown={(event) => {
                if (event.key === "ArrowDown") moveOptionFocus(event, (index + 1) % options.length);
                else if (event.key === "ArrowUp") moveOptionFocus(event, (index - 1 + options.length) % options.length);
                else if (event.key === "Home") moveOptionFocus(event, 0);
                else if (event.key === "End") moveOptionFocus(event, options.length - 1);
                else if (event.key === "Enter" || event.key === " ") {
                  event.preventDefault();
                  selectOption(index);
                } else if (event.key === "Escape") {
                  event.preventDefault();
                  close(true);
                } else if (event.key === "Tab") close();
              }}
            >
              <span className="atsumi-dropdown-check" aria-hidden="true">
                {option.value === value ? <FluentIcon glyph="\uE73E" /> : null}
              </span>
              <span>{option.label}</span>
            </button>
          ))}
        </div>,
        portalHost,
      ) : null}
    </div>
  );
}
