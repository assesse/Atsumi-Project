import { useEffect, useState } from "react";

const clamp = (value: number, min: number, max: number) => Math.min(max, Math.max(min, value));
export type ReplayLayout = {
  width: number; height: number; videoWidth: number; videoHeight: number;
  chatSize: number; stacked: boolean; padding: number; border: number;
};

/** Fit the actual picture, not an arbitrary 1600x920 box. No crop or stretch.
 * Compare a readable side rail with a bottom rail; prefer the side rail on ties.
 * This is independent of rendered geometry, so resizing cannot feed back into itself.
 */
export function fitReplayLayout(viewportWidth: number, viewportHeight: number, mediaAspect: number, expanded = false): ReplayLayout {
  const width = Math.max(4, Number.isFinite(viewportWidth) ? viewportWidth : 1024);
  const height = Math.max(4, Number.isFinite(viewportHeight) ? viewportHeight : 768);
  const aspect = Number.isFinite(mediaAspect) && mediaAspect >= .05 && mediaAspect <= 20 ? mediaAspect : 16 / 9;
  const padding = expanded ? 0 : Math.min(width <= 780 ? 6 : 18, (Math.min(width, height) - 4) / 2);
  const border = expanded ? 0 : 1;
  const availableWidth = width - 2 * (padding + border), availableHeight = height - 2 * (padding + border);
  const sideChat = Math.min(clamp(availableWidth * .22, 260, 353), availableWidth * .45);
  const sideHeight = Math.min(availableHeight, (availableWidth - sideChat) / aspect);
  const sideWidth = sideHeight * aspect;
  const bottomChat = Math.min(clamp(availableHeight * .27, 160, 240), availableHeight * .45);
  const stackedHeight = Math.min(availableWidth / aspect, availableHeight - bottomChat);
  const stackedWidth = stackedHeight * aspect;
  // A very short/wide picture must not reduce chat to a few unreadable lines.
  const stacked = sideChat < 220 || sideHeight < 160 || stackedWidth * stackedHeight > sideWidth * sideHeight * 1.02;
  return stacked
    ? { width: Math.max(stackedWidth, Math.min(availableWidth, 260)), height: availableHeight,
      videoWidth: stackedWidth, videoHeight: stackedHeight, chatSize: availableHeight - stackedHeight, stacked, padding, border }
    : { width: sideWidth + sideChat, height: sideHeight,
      videoWidth: sideWidth, videoHeight: sideHeight, chatSize: sideChat, stacked, padding, border };
}

/** Only mounted during replay. At most one cheap viewport update per frame;
 * no image work, DOM measurement loops, or global listeners on other tabs.
 */
export function useReplayLayout(aspect: number, expanded: boolean): ReplayLayout {
  const [viewport, setViewport] = useState(() => ({ width: window.innerWidth, height: window.innerHeight }));
  useEffect(() => {
    let frame: number | undefined;
    const resize = () => {
      if (frame !== undefined) return;
      frame = requestAnimationFrame(() => {
        frame = undefined;
        const width = window.innerWidth, height = window.innerHeight;
        setViewport(previous => previous.width === width && previous.height === height ? previous : { width, height });
      });
    };
    window.addEventListener("resize", resize);
    return () => { window.removeEventListener("resize", resize); if (frame !== undefined) cancelAnimationFrame(frame); };
  }, []);
  return fitReplayLayout(viewport.width, viewport.height, aspect, expanded);
}
