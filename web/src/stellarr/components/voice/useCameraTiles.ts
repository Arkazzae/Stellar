import { createContext, useCallback, useMemo, useState } from "react";

type ObserveTile = (tile: HTMLElement, userId: string) => () => void;

export const CameraTileContext = createContext<ObserveTile | null>(null);

/** A person may have more than one mounted tile. Keep receiving while any is visible. */
export function useCameraTiles() {
  const [visible, setVisible] = useState<ReadonlyMap<HTMLElement, string>>(() => new Map());
  const observe = useCallback<ObserveTile>((tile, userId) => {
    const doc = tile.ownerDocument;
    const Observer = doc.defaultView?.IntersectionObserver;
    let intersects = !Observer;
    let active = true;
    const update = () => {
      const wanted = active && intersects && !doc.hidden;
      setVisible((previous) => {
        if (previous.has(tile) === wanted) return previous;
        const next = new Map(previous);
        if (wanted) next.set(tile, userId);
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
  const cameras = useMemo(() => new Set(visible.values()), [visible]);
  return { cameras, observe };
}
