import { createContext, useCallback, useMemo, useState } from "react";

type ObserveTile = (tile: HTMLElement, userId: string, large: boolean) => () => void;

export const CameraTileContext = createContext<ObserveTile | null>(null);

/** A person may have more than one mounted tile. Keep receiving while any is visible. */
export function useCameraTiles() {
  const [visible, setVisible] = useState<ReadonlyMap<HTMLElement, { userId: string; large: boolean }>>(() => new Map());
  const observe = useCallback<ObserveTile>((tile, userId, large) => {
    const doc = tile.ownerDocument;
    const Observer = doc.defaultView?.IntersectionObserver;
    let intersects = !Observer;
    let active = true;
    const update = () => {
      const wanted = active && intersects && !doc.hidden;
      setVisible((previous) => {
        if (previous.has(tile) === wanted) return previous;
        const next = new Map(previous);
        if (wanted) next.set(tile, { userId, large });
        else next.delete(tile);
        return next;
      });
    };
    const observer = Observer && new Observer((entries) => {
      if (!active) return;
      const entry = entries[entries.length - 1];
      intersects = !!entry && entry.isIntersecting && entry.intersectionRatio > 0;
      update();
    }, { threshold: [0, 0.001] });
    observer?.observe(tile);
    doc.addEventListener("visibilitychange", update);
    if (!observer) update();
    return () => {
      active = false;
      observer?.disconnect();
      doc.removeEventListener("visibilitychange", update);
      update();
    };
  }, []);
  return useMemo(() => {
    const cameras = new Set<string>();
    const largeCameras = new Set<string>();
    for (const { userId, large } of visible.values()) {
      cameras.add(userId);
      if (large) largeCameras.add(userId);
    }
    return { cameras, largeCameras, observe };
  }, [visible, observe]);
}
