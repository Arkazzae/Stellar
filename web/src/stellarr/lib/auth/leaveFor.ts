/**
 * A full navigation that replaces this page in the history, for places
 * outside the web client's router, such as Cockpit under `/cockpit`, which is
 * a document of its own. Callers pass a path checked to stay on this origin.
 */
export function leaveFor(path: string): void {
  window.location.replace(path);
}
