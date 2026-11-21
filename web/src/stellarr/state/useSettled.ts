import { useState } from "react";

/**
 * The previous value for as long as `same` says nothing changed, so whatever
 * is built on it keeps its identity too.
 */
export function useSettled<T>(value: T, same: (a: T, b: T) => boolean): T {
  const [settled, setSettled] = useState(value);
  if (settled === value || same(settled, value)) return settled;
  setSettled(value);
  return value;
}
