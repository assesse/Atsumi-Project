import { useEffect } from "react";
import { PhysicalPosition, PhysicalSize } from "@tauri-apps/api/dpi";
import { availableMonitors, currentMonitor, getCurrentWindow } from "@tauri-apps/api/window";
import { backend } from "../api/backend";
import type { WindowPlacement, WindowPlacementSnapshot } from "../api/contracts";
import { fitWindowToWorkArea } from "../layout/windowWorkArea";

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
          // Maximized bounds are not the user's normal restore rectangle.
          x: maximized ? placement.x : position.x,
          y: maximized ? placement.y : position.y,
          width: maximized ? placement.width : size.width,
          height: maximized ? placement.height : size.height,
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
      const [monitors, current, outer, inner] = await Promise.all([
        availableMonitors(), currentMonitor(), appWindow.outerSize(), appWindow.innerSize(),
      ]);
      const fallback = current ?? monitors[0];
      if (!fallback || disposed) return;
      const fitted = fitWindowToWorkArea(placement, monitors, fallback, {
        width: outer.width - inner.width, height: outer.height - inner.height,
      });
      // Lower the native minimum when DPI scaling leaves a smaller work area.
      await appWindow.setMinSize(new PhysicalSize(fitted.minimum.width, fitted.minimum.height));
      await appWindow.setSize(new PhysicalSize(fitted.inner.width, fitted.inner.height));
      await appWindow.setPosition(new PhysicalPosition(fitted.position.x, fitted.position.y));
      placement = { ...placement, ...fitted.position, ...fitted.outer };
      if (placement.maximized) await appWindow.maximize();
      for (const listen of [appWindow.onMoved.bind(appWindow), appWindow.onResized.bind(appWindow)]) {
        const unlisten = await listen(schedule);
        if (disposed) unlisten(); else unlisteners.push(unlisten);
      }
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
