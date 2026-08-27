import { useEffect } from "react";
import { PhysicalPosition, PhysicalSize } from "@tauri-apps/api/dpi";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { backend } from "../api/backend";
import type { WindowPlacement, WindowPlacementSnapshot } from "../api/contracts";

export function useWindowPlacement(): void {
  useEffect(() => {
    if (backend.runtime !== "tauri") return;

    let disposed = false;
    let placement: WindowPlacementSnapshot | null = null;
    let timer: number | undefined;
    const unlisteners: Array<() => void> = [];
    const appWindow = getCurrentWindow();

    const persist = async () => {
      if (!placement || disposed) return;
      try {
        const [position, size, maximized] = await Promise.all([
          appWindow.outerPosition(),
          appWindow.outerSize(),
          appWindow.isMaximized(),
        ]);
        const next: WindowPlacement = {
          x: position.x,
          y: position.y,
          width: size.width,
          height: size.height,
          maximized,
        };
        const result = await backend.windowPlacementUpdate(next, placement.revision);
        if (result.ok) placement = result.data;
        else if (result.error.code === "REVISION_CONFLICT") {
          const refreshed = await backend.windowPlacementGet();
          if (refreshed.ok) placement = refreshed.data;
        }
      } catch {
        // A later move/resize event retries persistence without interrupting the UI.
      }
    };

    const schedule = () => {
      window.clearTimeout(timer);
      timer = window.setTimeout(() => void persist(), 250);
    };

    void (async () => {
      const result = await backend.windowPlacementGet();
      if (!result.ok || disposed) return;
      placement = result.data;
      if (placement.x !== null && placement.y !== null) {
        await appWindow.setPosition(new PhysicalPosition(placement.x, placement.y));
      }
      await appWindow.setSize(new PhysicalSize(placement.width, placement.height));
      if (placement.maximized) await appWindow.maximize();
      unlisteners.push(await appWindow.onMoved(schedule));
      unlisteners.push(await appWindow.onResized(schedule));
    })().catch(() => {
      // Keep the default Tauri placement when restore is unavailable.
    });

    return () => {
      disposed = true;
      window.clearTimeout(timer);
      unlisteners.forEach((unlisten) => unlisten());
    };
  }, []);
}
