import { useEffect, useEffectEvent, useRef, type RefObject } from "react";

/**
 * How close to the top of the loaded history the reader gets before the page
 * before it is asked for. Generous, so someone scrolling up at speed meets
 * history that is already there rather than a pause at the top.
 */
export const OLDER_MARGIN_PX = 900;

interface Options {
  scroller: RefObject<HTMLElement | null>;
  /** The oldest loaded message block; it changes when a page lands. */
  oldestId: string | null;
  hasOlder: boolean;
  loadingOlder: boolean;
  onLoadOlder: (() => void) | undefined;
}

/**
 * Asks for the next backwards page when the reader nears the top.
 *
 * Asked on the way into the top margin, and again after a page lands if the
 * top is still within reach: a first page that does not fill the pane, or a
 * short one after it, would otherwise leave nothing to scroll and no way to
 * reach more. It is not asked again from the same oldest block, because a page
 * that failed leaves the rows as they were, and retrying on every settle would
 * hammer a server that is already failing; the next trip into the margin asks
 * again. The explicit load control can retry even when the loaded history is
 * too short to leave the margin.
 */
export function useOlderHistory({
  scroller,
  oldestId,
  hasOlder,
  loadingOlder,
  onLoadOlder,
}: Options) {
  /** The oldest block when a page was last asked for; null when it may be asked. */
  const askedAt = useRef<{ oldest: string | null } | null>(null);
  const near = useRef(false);

  const loadOlder = () => {
    if (!hasOlder || loadingOlder || onLoadOlder === undefined) return;
    askedAt.current = { oldest: oldestId };
    onLoadOlder();
  };

  const ask = useEffectEvent(() => {
    if (askedAt.current !== null && askedAt.current.oldest === oldestId) return;
    loadOlder();
  });

  useEffect(() => {
    const element = scroller.current;
    if (element === null) return;
    const onScroll = () => {
      // A hidden transcript measures zero and would read as being at the top.
      if (element.clientHeight === 0) return;
      const isNear = element.scrollTop <= OLDER_MARGIN_PX;
      if (isNear && !near.current) askedAt.current = null;
      near.current = isNear;
      if (isNear) ask();
    };
    element.addEventListener("scroll", onScroll, { passive: true });
    return () => element.removeEventListener("scroll", onScroll);
  }, [scroller]);

  // After the commit that changed the rows has put the reader back in place,
  // which is a layout effect and so already done by now.
  useEffect(() => {
    const element = scroller.current;
    if (element === null || element.clientHeight === 0) return;
    near.current = element.scrollTop <= OLDER_MARGIN_PX;
    if (near.current) ask();
  }, [scroller, oldestId, hasOlder, loadingOlder, onLoadOlder]);

  return loadOlder;
}
