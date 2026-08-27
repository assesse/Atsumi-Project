import { useLayoutEffect, useState, type RefObject } from "react";
import { createPortal } from "react-dom";
import type { TagTooltipLanguage } from "../data/tagTranslations";

type TagTranslationTooltipProps = {
  id: string;
  trigger: RefObject<HTMLElement | null>;
  open: boolean;
  text: string;
  language: TagTooltipLanguage;
};

export function TagTranslationTooltip({ id, trigger, open, text, language }: TagTranslationTooltipProps) {
  const [tooltip, setTooltip] = useState<HTMLSpanElement | null>(null);
  const [position, setPosition] = useState({ left: -10_000, top: -10_000 });

  useLayoutEffect(() => {
    if (!open || !trigger.current || !tooltip) return;
    const triggerRect = trigger.current.getBoundingClientRect();
    const tooltipRect = tooltip.getBoundingClientRect();
    setPosition({
      left: Math.max(8, Math.min(triggerRect.left + triggerRect.width / 2 - tooltipRect.width / 2, window.innerWidth - tooltipRect.width - 8)),
      top: Math.max(8, triggerRect.top - tooltipRect.height - 8),
    });
  }, [open, text, tooltip, trigger]);

  if (!open) return null;
  return createPortal(
    <span
      ref={setTooltip}
      id={id}
      role="tooltip"
      className="tag-translation-tooltip"
      lang={language}
      style={position}
    >
      {text}
    </span>,
    document.body,
  );
}
