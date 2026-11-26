import { useCallback, useEffect, useRef, useState } from "react";

/** How long the pointer may rest before the controls on a picture fade. */
export const CHROME_IDLE_MS = 2500;

/**
 * Whether the controls laid over a picture should fade: true once the pointer
 * has rested on the stage for a while. Any movement, press or key brings them
 * back; a focused control keeps them up through CSS (`:focus-within`), so a
 * keyboard user never loses the button they are on.
 */
export function useChromeIdle(restMs: number = CHROME_IDLE_MS) {
  const [idle, setIdle] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);

  const wake = useCallback(() => {
    setIdle(false);
    if (timer.current !== null) clearTimeout(timer.current);
    timer.current = setTimeout(() => setIdle(true), restMs);
  }, [restMs]);

  useEffect(
    () => () => {
      if (timer.current !== null) clearTimeout(timer.current);
    },
    [],
  );

  return { idle, wake };
}
