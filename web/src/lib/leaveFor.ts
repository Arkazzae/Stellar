/**
 * A full navigation that replaces this page in the history, for places
 * outside the web client's layout, such as Cockpit under `/cockpit`, which has
 * a root layout of its own. Callers pass a path checked to stay on this
 * origin.
 */
export function leaveFor(path: string): void {
  window.location.replace(path);
}
