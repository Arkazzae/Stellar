import { useCallback, useLayoutEffect, useRef } from "react";

/**
 * An event handler that keeps its identity for the component's whole life and
 * always runs the latest `handler`, so it can go to a memoised child without
 * rendering it. Undefined while `handler` is, which is the one change the
 * child does see.
 *
 * For events only: the latest handler is taken once the render is committed,
 * so calling the result during a render would run the previous one.
 */
export function useStableHandler<A extends unknown[], R>(
  handler: (...args: A) => R,
): (...args: A) => R;
export function useStableHandler<A extends unknown[], R>(
  handler: ((...args: A) => R) | undefined,
): ((...args: A) => R) | undefined;
export function useStableHandler<A extends unknown[], R>(
  handler: ((...args: A) => R) | undefined,
): ((...args: A) => R) | undefined {
  const latest = useRef(handler);
  useLayoutEffect(() => {
    latest.current = handler;
  });
  // A child that still holds it after the handler went away does nothing.
  const stable = useCallback((...args: A) => latest.current?.(...args) as R, []);
  return handler === undefined ? undefined : stable;
}
