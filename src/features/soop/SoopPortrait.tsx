import { useEffect, useRef, useState } from "react";
import type { SoopApi } from "../../api/soop";
import { subscribeSoopPortrait } from "./soopImageQueue";

export function SoopPortrait({ api, channelId, active, privacy, className = "soop-channel-portrait" }: {
  api: SoopApi; channelId: string; active: boolean; privacy: boolean; className?: string;
}) {
  const target = useRef<HTMLDivElement>(null);
  const [image, setImage] = useState<{ id: string; value: string } | null>(null);
  const [failed, setFailed] = useState("");
  useEffect(() => {
    setImage(null); setFailed("");
    if (!active || privacy) return;
    let cancelled = false;
    let unsubscribe: (() => void) | undefined;
    const load = () => {
      if (cancelled || unsubscribe) return;
      unsubscribe = subscribeSoopPortrait(api, channelId, value => { if (!cancelled) setImage({ id: channelId, value }); });
    };
    const observer = typeof IntersectionObserver === "undefined" ? null : new IntersectionObserver(entries => {
      if (entries.some(entry => entry.isIntersecting)) { observer!.disconnect(); load(); }
    }, { rootMargin: "100px" });
    if (observer && target.current) observer.observe(target.current); else load();
    return () => { cancelled = true; observer?.disconnect(); unsubscribe?.(); };
  }, [api.inspect, channelId, active, privacy]);
  const src = active && !privacy && image?.id === channelId && image.value !== failed ? image.value : "";
  return <div ref={target} className={className} aria-hidden="true">{src ? <img src={src} alt="" onError={() => setFailed(src)} /> : <span>○</span>}</div>;
}
