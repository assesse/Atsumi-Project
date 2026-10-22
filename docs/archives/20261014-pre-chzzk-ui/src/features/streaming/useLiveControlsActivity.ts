import { useEffect, useState } from "react";
/** Native child WebViews do not bubble pointer events into React's document. */
export function useLiveControlsActivity(runtime: string) {
  const [active, setActive] = useState(true);
  useEffect(() => {
    let disposed = false, timer: ReturnType<typeof setTimeout>, unlisten: (() => void) | undefined;
    const reveal = () => { if (disposed) return; setActive(true); clearTimeout(timer); timer = setTimeout(() => setActive(false), 2400); };
    window.addEventListener("pointermove", reveal, { passive: true });
    window.addEventListener("keydown", reveal); reveal();
    if (runtime === "tauri") void import("@tauri-apps/api/event").then(({ listen }) => listen<{ action: string }>("chzzk-presentation", event => { if (event.payload.action === "pointer_activity") reveal(); }))
      .then(stop => { if (disposed) stop(); else unlisten = stop; }).catch(() => {});
    return () => { disposed = true; clearTimeout(timer); unlisten?.(); window.removeEventListener("pointermove", reveal); window.removeEventListener("keydown", reveal); };
  }, [runtime]);
  return active;
}
