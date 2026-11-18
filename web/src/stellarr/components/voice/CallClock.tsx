import { useEffect, useState } from "react";

/**
 * Whole seconds since `since`.
 *
 * It starts at 0 and only then follows the clock, because reading `Date.now()`
 * during render would differ between two passes. Each tick is scheduled for
 * the next whole second of the call rather than a second after the last one,
 * so two clocks of the same call (the dock's and the call view's) turn over
 * together instead of a fraction of a second apart.
 */
function useElapsedSeconds(since: number): number {
  const [seconds, setSeconds] = useState(0);

  useEffect(() => {
    let timer = 0;
    const tick = () => {
      const elapsed = Math.max(0, Date.now() - since);
      setSeconds(Math.floor(elapsed / 1000));
      timer = window.setTimeout(tick, 1000 - (elapsed % 1000));
    };
    tick();
    return () => window.clearTimeout(timer);
  }, [since]);

  return seconds;
}

const twoDigits = (value: number) => String(value).padStart(2, "0");

/** How long a call has lasted, as minutes and seconds. */
export function CallClock({ since }: { since: number }) {
  const seconds = useElapsedSeconds(since);
  const minutes = Math.floor(seconds / 60);
  const rest = seconds % 60;

  return (
    <time dateTime={`PT${minutes}M${rest}S`}>{`${twoDigits(minutes)}:${twoDigits(rest)}`}</time>
  );
}
