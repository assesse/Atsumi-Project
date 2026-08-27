import { forwardRef, useEffect, useId, useRef, useState, type ForwardedRef, type MouseEvent, type MouseEventHandler } from "react";
import { tagTooltip } from "../data/tagTranslations";
import { metadataSearchToken } from "../search/searchTokens";
import { TagTranslationTooltip } from "./TagTranslationTooltip";

type MetadataIconProps = {
  kind: "artist" | "group";
};

// Exact 20px Regular geometry vendored from Microsoft Fluent UI System Icons (MIT).
// https://github.com/microsoft/fluentui-system-icons/tree/1.1.328/assets
const metadataIconPath = {
  artist: "M10 2C7.79086 2 6 3.79086 6 6C6 8.20914 7.79086 10 10 10C12.2091 10 14 8.20914 14 6C14 3.79086 12.2091 2 10 2ZM7 6C7 4.34315 8.34315 3 10 3C11.6569 3 13 4.34315 13 6C13 7.65685 11.6569 9 10 9C8.34315 9 7 7.65685 7 6ZM5.00873 11C3.90315 11 3 11.8869 3 13C3 14.6912 3.83281 15.9663 5.13499 16.7966C6.41697 17.614 8.14526 18 10 18C11.8547 18 13.583 17.614 14.865 16.7966C16.1672 15.9663 17 14.6912 17 13C17 11.8956 16.1045 11 15 11L5.00873 11ZM4 13C4 12.4467 4.44786 12 5.00873 12L15 12C15.5522 12 16 12.4478 16 13C16 14.3088 15.3777 15.2837 14.3274 15.9534C13.2568 16.636 11.7351 17 10 17C8.26489 17 6.74318 16.636 5.67262 15.9534C4.62226 15.2837 4 14.3088 4 13Z",
  group: "M4.5 6.75C4.5 5.50736 5.50736 4.5 6.75 4.5C7.99264 4.5 9 5.50736 9 6.75C9 7.99264 7.99264 9 6.75 9C5.50736 9 4.5 7.99264 4.5 6.75ZM6.75 3.5C4.95507 3.5 3.5 4.95507 3.5 6.75C3.5 8.54493 4.95507 10 6.75 10C8.54493 10 10 8.54493 10 6.75C10 4.95507 8.54493 3.5 6.75 3.5ZM12.4368 15.1453C12.9748 15.3644 13.6518 15.5 14.4995 15.5C16.381 15.5 17.4213 14.8322 17.9689 14.0656C18.2335 13.6953 18.3653 13.3257 18.4313 13.0486C18.4644 12.9096 18.4814 12.7919 18.4901 12.706C18.4945 12.663 18.4969 12.6277 18.4981 12.6013C18.4987 12.5881 18.4991 12.5771 18.4993 12.5685L18.4995 12.5574L18.4995 12.5533L18.4995 12.5515L18.4995 12.55V12.5C18.4995 11.6716 17.828 11 16.9995 11H12.3711C12.6102 11.2895 12.7912 11.6288 12.8962 12H16.9995C17.2757 12 17.4995 12.2239 17.4995 12.5V12.5462L17.4993 12.5537C17.4988 12.5633 17.4977 12.5806 17.4953 12.6045C17.4904 12.6526 17.48 12.7263 17.4585 12.817C17.4151 12.9993 17.3281 13.2422 17.1552 13.4844C16.8277 13.9428 16.1181 14.5 14.4995 14.5C13.7679 14.5 13.222 14.3862 12.8132 14.219C12.7312 14.4984 12.6116 14.8153 12.4368 15.1453ZM1.5 13C1.5 11.8954 2.39543 11 3.5 11H10C11.1046 11 12 11.8954 12 13V13.0625L12 13.064L12 13.0658L12 13.0705L11.9997 13.0835C11.9995 13.0938 11.9991 13.1074 11.9983 13.1241C11.9968 13.1574 11.9939 13.2031 11.9883 13.2593C11.9772 13.3716 11.9555 13.5272 11.913 13.7118C11.8282 14.08 11.6586 14.5719 11.3176 15.0655C10.6166 16.0801 9.26315 17 6.75 17C4.23685 17 2.8834 16.0801 2.18238 15.0655C1.8414 14.5719 1.67175 14.08 1.58697 13.7118C1.54446 13.5272 1.52278 13.3716 1.5117 13.2593C1.50614 13.2031 1.50322 13.1574 1.50169 13.1241C1.50092 13.1074 1.5005 13.0938 1.50027 13.0835L1.50005 13.0705L1.50001 13.0658L1.5 13.064L1.5 13.0625V13ZM2.5 13.0602L2.50002 13.0612L2.50063 13.0781C2.50141 13.0951 2.50313 13.1233 2.50686 13.1611C2.51433 13.2368 2.52976 13.3497 2.56146 13.4874C2.62512 13.7638 2.75235 14.1312 3.00512 14.497C3.4916 15.2012 4.51315 16 6.75 16C8.98685 16 10.0084 15.2012 10.4949 14.497C10.7477 14.1312 10.8749 13.7638 10.9385 13.4874C10.9702 13.3497 10.9857 13.2368 10.9931 13.1611C10.9969 13.1233 10.9986 13.0951 10.9994 13.0781L11 13.0612L11 13.0602V13C11 12.4477 10.5523 12 10 12H3.5C2.94772 12 2.5 12.4477 2.5 13V13.0602ZM13 7.5C13 6.67157 13.6716 6 14.5 6C15.3284 6 16 6.67157 16 7.5C16 8.32843 15.3284 9 14.5 9C13.6716 9 13 8.32843 13 7.5ZM14.5 5C13.1193 5 12 6.11929 12 7.5C12 8.88071 13.1193 10 14.5 10C15.8807 10 17 8.88071 17 7.5C17 6.11929 15.8807 5 14.5 5Z",
} as const;

function MetadataIcon({ kind }: MetadataIconProps) {
  return (
    <svg
      className="metadata-chip-icon"
      data-metadata-icon={kind}
      data-fluent-icon={kind === "artist" ? "person-20-regular" : "people-20-regular"}
      viewBox="0 0 20 20"
      width="16"
      height="16"
      fill="currentColor"
      aria-hidden="true"
      focusable="false"
    >
      <path d={metadataIconPath[kind]} fill="currentColor" stroke="none" />
    </svg>
  );
}

type MetadataChipProps = {
  value: string;
  searchValue?: string;
  label?: string;
  favorite?: boolean;
  kind?: "meta-chip" | "tag" | "byline";
  onClickCapture?: MouseEventHandler<HTMLButtonElement>;
  onSearch: (value: string) => void;
  onToggleFavorite: (value: string) => void;
};

export const MetadataChip = forwardRef<HTMLButtonElement, MetadataChipProps>(function MetadataChip({
  value,
  searchValue,
  label,
  favorite = false,
  kind = "meta-chip",
  onClickCapture,
  onSearch,
  onToggleFavorite,
}, ref) {
  const triggerRef = useRef<HTMLButtonElement>(null);
  const tooltipTimer = useRef<number | undefined>(undefined);
  const tooltipId = useId();
  const [tooltipOpen, setTooltipOpen] = useState(false);
  const [namespace = "", ...rest] = value.split(":");
  const visibleLabel = label ?? (rest.length ? rest.join(":") : namespace).replaceAll("_", " ");
  const namespaceClass = ["female", "male", "artist", "group"].includes(namespace) ? namespace : "";
  const classes = [kind, namespaceClass, favorite ? "favorite" : ""].filter(Boolean).join(" ");
  const tagNamespaceLabel = namespace === "female"
    ? "Female 태그"
    : namespace === "male"
      ? "Male 태그"
      : "중립 태그";
  const accessibleLabel = kind === "tag"
    ? `${visibleLabel}, ${tagNamespaceLabel}${favorite ? ", 즐겨찾기" : ""}, 좌클릭 검색, 우클릭 즐겨찾기 변경`
    : `${visibleLabel}${favorite ? ", 즐겨찾기" : ""}, 좌클릭 검색, 우클릭 즐겨찾기 변경`;
  const translation = kind === "tag" ? tagTooltip(value) : undefined;

  useEffect(() => () => {
    if (tooltipTimer.current !== undefined) window.clearTimeout(tooltipTimer.current);
  }, []);

  const setTriggerRef = (node: HTMLButtonElement | null) => {
    triggerRef.current = node;
    const forwarded = ref as ForwardedRef<HTMLButtonElement>;
    if (typeof forwarded === "function") forwarded(node);
    else if (forwarded) forwarded.current = node;
  };

  const showTooltip = (delayed: boolean) => {
    if (!translation) return;
    if (tooltipTimer.current !== undefined) window.clearTimeout(tooltipTimer.current);
    if (!delayed) {
      setTooltipOpen(true);
      return;
    }
    tooltipTimer.current = window.setTimeout(() => {
      tooltipTimer.current = undefined;
      setTooltipOpen(true);
    }, 240);
  };

  const hideTooltip = () => {
    if (tooltipTimer.current !== undefined) window.clearTimeout(tooltipTimer.current);
    tooltipTimer.current = undefined;
    setTooltipOpen(false);
  };

  const handleContextMenu = (event: MouseEvent<HTMLButtonElement>) => {
    event.preventDefault();
    event.stopPropagation();
    onToggleFavorite(value);
  };

  return (
    <button
      type="button"
      ref={setTriggerRef}
      className={classes}
      aria-label={accessibleLabel}
      aria-describedby={tooltipOpen && translation ? tooltipId : undefined}
      data-tag-tooltip-language={translation?.language}
      data-tag-translation-key={translation?.key}
      title={kind === "tag" ? undefined : `${visibleLabel} · 좌클릭 검색 / 우클릭 즐겨찾기`}
      onClickCapture={onClickCapture}
      onMouseEnter={() => showTooltip(true)}
      onMouseLeave={hideTooltip}
      onFocus={() => showTooltip(false)}
      onBlur={hideTooltip}
      onKeyDown={(event) => {
        if (event.key === "Escape") hideTooltip();
      }}
      onClick={(event) => {
        event.stopPropagation();
        onSearch(kind === "tag" ? metadataSearchToken(value, searchValue).displayToken : (searchValue ?? value));
      }}
      onContextMenu={handleContextMenu}
    >
      {kind === "byline" && namespace === "artist" ? <MetadataIcon kind="artist" /> : null}
      {kind === "byline" && namespace === "group" ? <MetadataIcon kind="group" /> : null}
      {kind === "tag" && (namespace === "female" || namespace === "male") ? (
        <span className="tag-namespace" aria-hidden="true">{namespace === "female" ? "F" : "M"}</span>
      ) : null}
      <span className={kind === "tag" ? "tag-label" : undefined}>{visibleLabel}</span>
      {kind === "tag" && favorite ? <span className="tag-favorite" aria-hidden="true">★</span> : null}
      {translation ? <TagTranslationTooltip id={tooltipId} trigger={triggerRef} open={tooltipOpen} text={translation.text} language={translation.language} /> : null}
    </button>
  );
});
