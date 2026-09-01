import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
  type CSSProperties,
  type HTMLAttributes,
  type ReactNode,
  type Ref,
} from "react";
import {
  ThumbnailClient,
  useThumbnailClient,
  type ThumbnailAsset,
  type ThumbnailConsumer,
  type ThumbnailKey,
  type ThumbnailPriority,
  type ThumbnailSnapshot,
  type ThumbnailSpriteAsset,
  thumbnailKeyIdentity,
} from "../thumbnail";

type GalleryThumbnailProps = Omit<HTMLAttributes<HTMLElement>, "children"> & {
  thumbnailKey: ThumbnailKey;
  consumer: ThumbnailConsumer;
  priority: ThumbnailPriority;
  alt: string;
  /** Use a span when this media is nested in a button. */
  as?: "div" | "span";
  sizing?: "container" | "intrinsic";
  /** Reserve this ratio before the coordinator returns intrinsic image dimensions. */
  expectedAspectRatio?: { readonly width: number; readonly height: number };
  /** Receives terminal thumbnail states without creating a second subscription. */
  onTerminalSnapshot?: (snapshot: {
    status: "resolved" | "error";
    width?: number;
    height?: number;
    kind?: ThumbnailAsset["kind"];
  }) => void;
  rootRef?: Ref<HTMLElement>;
  client?: ThumbnailClient;
  children?: ReactNode;
};

const fullBleedStyle: CSSProperties = {
  position: "absolute",
  inset: 0,
  display: "block",
  width: "100%",
  height: "100%",
};

const deferredSnapshot: ThumbnailSnapshot = { status: "idle" };
const nearViewportMargin = "600px 0px";
const detailViewportMargin = "260px 0px";

const loadingForPriority = (priority: ThumbnailPriority): "eager" | "lazy" =>
  priority === "prefetch" ? "lazy" : "eager";

const assignRef = <T,>(ref: Ref<T> | undefined, value: T | null): void => {
  if (typeof ref === "function") ref(value);
  else if (ref) (ref as { current: T | null }).current = value;
};

const thumbnailFailureLabel = (code: string | undefined): string => {
  switch (code) {
    case "THUMBNAIL_notFound":
    case "THUMBNAIL_candidatesExhausted":
      return "원본 이미지를 찾을 수 없음";
    case "THUMBNAIL_responseInvalid":
      return "이미지가 아닌 응답을 받음";
    case "THUMBNAIL_decodeFailed":
      return "이미지를 안전하게 처리할 수 없음";
    case "THUMBNAIL_unauthorized":
      return "원본 사이트에서 접근을 거부함";
    case "THUMBNAIL_temporarilyUnavailable":
    case "THUMBNAIL_resolver":
    case "THUMBNAIL_COMPLETION_TIMEOUT":
    case "THUMBNAIL_WORKER_UNAVAILABLE":
      return "미리보기를 일시적으로 불러올 수 없음";
    default:
      return "미리보기 불러오기 실패";
  }
};

const greatestCommonDivisor = (left: number, right: number): number => {
  let a = Math.abs(Math.round(left));
  let b = Math.abs(Math.round(right));
  while (b) [a, b] = [b, a % b];
  return a || 1;
};

const intrinsicAspectRatio = (
  asset: ThumbnailAsset | null,
  expectedAspectRatio?: { readonly width: number; readonly height: number },
): string => {
  if (
    expectedAspectRatio
    && Number.isFinite(expectedAspectRatio.width)
    && expectedAspectRatio.width > 0
    && Number.isFinite(expectedAspectRatio.height)
    && expectedAspectRatio.height > 0
  ) return `${expectedAspectRatio.width} / ${expectedAspectRatio.height}`;
  if (asset?.kind === "image") return `${asset.width} / ${asset.height}`;
  if (asset?.kind === "sprite") {
    const width = asset.sheetWidth * asset.rows;
    const height = asset.sheetHeight * asset.columns;
    const divisor = greatestCommonDivisor(width, height);
    return `${width / divisor} / ${height / divisor}`;
  }
  return "1 / 1";
};

const intrinsicDimensions = (
  asset: ThumbnailAsset | null,
  expectedAspectRatio?: { readonly width: number; readonly height: number },
): { width: number; height: number } => {
  if (
    expectedAspectRatio
    && Number.isFinite(expectedAspectRatio.width)
    && expectedAspectRatio.width > 0
    && Number.isFinite(expectedAspectRatio.height)
    && expectedAspectRatio.height > 0
  ) return { width: expectedAspectRatio.width, height: expectedAspectRatio.height };
  if (asset?.kind === "image") return { width: asset.width, height: asset.height };
  if (asset?.kind === "sprite") {
    return {
      width: asset.sheetWidth / asset.columns,
      height: asset.sheetHeight / asset.rows,
    };
  }
  return { width: 1, height: 1 };
};

function SpriteImage({ asset, alt, loading, onError }: {
  asset: ThumbnailSpriteAsset;
  alt: string;
  loading: "eager" | "lazy";
  onError: () => void;
}) {
  const column = asset.cell % asset.columns;
  const row = Math.floor(asset.cell / asset.columns);
  const cellWidth = asset.sheetWidth / asset.columns;
  const cellHeight = asset.sheetHeight / asset.rows;
  return (
    <i
      className="thumbnail-sprite-viewport"
      style={{
        position: "absolute",
        inset: 0,
        margin: "auto",
        display: "block",
        width: "100%",
        height: "auto",
        maxWidth: "100%",
        maxHeight: "100%",
        aspectRatio: `${cellWidth} / ${cellHeight}`,
        overflow: "hidden",
        fontStyle: "normal",
      }}
    >
      <img
        className="thumbnail-image thumbnail-image--sprite cover-image cover-image--sprite"
        src={asset.url}
        width={asset.sheetWidth}
        height={asset.sheetHeight}
        loading={loading}
        decoding="async"
        alt={alt}
        onError={onError}
        style={{
          position: "absolute",
          top: `${-row * 100}%`,
          left: `${-column * 100}%`,
          display: "block",
          width: `${asset.columns * 100}%`,
          height: `${asset.rows * 100}%`,
          maxWidth: "none",
          objectFit: "fill",
        }}
      />
    </i>
  );
}

function ThumbnailVisual({ asset, alt, loading, onError }: {
  asset: ThumbnailAsset;
  alt: string;
  loading: "eager" | "lazy";
  onError: () => void;
}) {
  if (asset.kind === "image") {
    return (
      <img
        className="thumbnail-image cover-image"
        src={asset.url}
        width={asset.width}
        height={asset.height}
        loading={loading}
        decoding="async"
        alt={alt}
        onError={onError}
        style={{ ...fullBleedStyle, objectFit: "contain", objectPosition: "center" }}
      />
    );
  }
  if (asset.kind === "sprite") {
    return <SpriteImage asset={asset} alt={alt} loading={loading} onError={onError} />;
  }
  return (
    <i
      className="thumbnail-fallback"
      role="img"
      aria-label={`${alt} 없음`}
      style={{ ...fullBleedStyle, display: "grid", placeItems: "center", color: "#64767d", fontStyle: "normal" }}
    >
      —
    </i>
  );
}

export function GalleryThumbnail({
  thumbnailKey,
  consumer,
  priority,
  alt,
  as = "div",
  sizing = "container",
  expectedAspectRatio,
  onTerminalSnapshot,
  rootRef,
  client: clientOverride,
  className,
  style,
  children,
  ...elementProps
}: GalleryThumbnailProps) {
  const client = useThumbnailClient(clientOverride);
  const identity = thumbnailKeyIdentity(thumbnailKey);
  const elementRef = useRef<HTMLElement | null>(null);
  const setElementRef = useCallback((element: HTMLElement | null) => {
    elementRef.current = element;
    assignRef(rootRef, element);
  }, [rootRef]);
  const hasIntersectionObserver = typeof IntersectionObserver === "function";
  const [activatedIdentity, setActivatedIdentity] = useState<string | null>(() =>
    priority === "critical" || !hasIntersectionObserver ? identity : null,
  );
  const shouldSubscribe = priority === "critical"
    || !hasIntersectionObserver
    || activatedIdentity === identity;
  const effectivePriority: ThumbnailPriority = priority === "critical"
    ? "critical"
    : hasIntersectionObserver && activatedIdentity === identity
      ? "visible"
      : priority;

  useEffect(() => {
    if (priority === "critical" || typeof IntersectionObserver !== "function") return undefined;
    const element = elementRef.current;
    if (!element) return undefined;
    const detailRoot = element.closest<HTMLElement>("[data-thumbnail-scroll-root]");
    const viewportRoot = detailRoot ?? element.closest<HTMLElement>(".gallery-viewport");
    const observer = new IntersectionObserver((entries) => {
      const visible = entries.some((entry) => entry.isIntersecting);
      setActivatedIdentity((current) => visible
        ? identity
        : current === identity ? null : current);
    }, {
      root: viewportRoot,
      rootMargin: detailRoot ? detailViewportMargin : nearViewportMargin,
      threshold: 0.01,
    });
    observer.observe(element);
    return () => observer.disconnect();
  }, [identity, priority]);

  const request = useMemo(
    () => ({ key: thumbnailKey, consumer, priority: effectivePriority } as const),
    [consumer, effectivePriority, thumbnailKey],
  );
  const subscribe = useCallback(
    (listener: () => void) => shouldSubscribe
      ? client.subscribe(request, listener)
      : () => undefined,
    [client, request, shouldSubscribe],
  );
  const getSnapshot = useCallback(
    () => shouldSubscribe ? client.getSnapshot(thumbnailKey) : deferredSnapshot,
    [client, shouldSubscribe, thumbnailKey],
  );
  const snapshot = useSyncExternalStore(subscribe, getSnapshot, getSnapshot);
  const asset = snapshot.status === "resolved" ? snapshot.asset : null;
  const state = !shouldSubscribe
    ? "deferred"
    : snapshot.status === "resolved"
      ? snapshot.asset.kind
      : snapshot.status;
  const loading = loadingForPriority(effectivePriority);
  const reportDisplayFailure = useCallback(
    () => client.reportDisplayFailure(request, "Resolved thumbnail could not be decoded"),
    [client, request],
  );
  const intrinsicStyle = sizing === "intrinsic"
    ? { aspectRatio: intrinsicAspectRatio(asset, expectedAspectRatio) }
    : undefined;
  const dimensions = intrinsicDimensions(asset, expectedAspectRatio);
  const Element = as;

  useEffect(() => {
    if (!onTerminalSnapshot || !shouldSubscribe) return;
    if (snapshot.status === "error") {
      onTerminalSnapshot({ status: "error" });
      return;
    }
    if (snapshot.status === "resolved") {
      const resolved = intrinsicDimensions(snapshot.asset, undefined);
      onTerminalSnapshot({
        status: "resolved",
        width: resolved.width,
        height: resolved.height,
        kind: snapshot.asset.kind,
      });
    }
  }, [onTerminalSnapshot, shouldSubscribe, snapshot]);

  return (
    <Element
      {...elementProps}
      ref={setElementRef}
      className={[
        className,
        "gallery-thumbnail",
        asset?.kind === "image" ? "has-thumbnail-image" : "has-sprite-image",
        `thumbnail-${state}`,
      ].filter(Boolean).join(" ")}
      data-thumbnail-kind={thumbnailKey.kind}
      data-thumbnail-consumer={consumer}
      data-thumbnail-priority={effectivePriority}
      data-thumbnail-state={state}
      data-thumbnail-intrinsic-width={dimensions.width}
      data-thumbnail-intrinsic-height={dimensions.height}
      aria-busy={shouldSubscribe && (snapshot.status === "idle" || snapshot.status === "loading") || undefined}
      style={{
        position: "relative",
        overflow: "hidden",
        backgroundColor: "var(--thumbnail-frame)",
        backgroundImage: "none",
        ...intrinsicStyle,
        ...style,
      }}
    >
      {!shouldSubscribe ? (
        <i className="thumbnail-deferred" aria-hidden="true" />
      ) : asset ? (
        <ThumbnailVisual asset={asset} alt={alt} loading={loading} onError={reportDisplayFailure} />
      ) : snapshot.status === "error" ? (
        <i
          className="thumbnail-fallback thumbnail-fallback--error"
          role="img"
          aria-label={`${alt} ${thumbnailFailureLabel(snapshot.code)}`}
          title={snapshot.message}
          style={{ ...fullBleedStyle, display: "grid", placeItems: "center", color: "#64767d", fontStyle: "normal" }}
        >
          ×
        </i>
      ) : (
        <i className="thumbnail-loading" aria-hidden="true" />
      )}
      {children}
    </Element>
  );
}
