import { useSyncExternalStore } from "react";

/**
 * Whether the dock over a full-screen activity always comes up at the bottom
 * edge, whatever the activity asks with `shell.dock.hold()`. It is the
 * player's answer and stays in this browser, as `{"always":true}`; absent means
 * an activity may hold the edge.
 */
export const DOCK_PREFERENCE_KEY = "stellarr.activityDock";

const listeners = new Set<() => void>();
let always: boolean | undefined;

function readStored(): boolean {
  try {
    const raw = window.localStorage.getItem(DOCK_PREFERENCE_KEY);
    return raw !== null && (JSON.parse(raw) as { always?: unknown } | null)?.always === true;
  } catch {
    return false;
  }
}

function notify() {
  for (const listener of listeners) listener();
}

export function dockAlways(): boolean {
  return (always ??= readStored());
}

export function setDockAlways(value: boolean): void {
  always = value;
  try {
    if (value) window.localStorage.setItem(DOCK_PREFERENCE_KEY, JSON.stringify({ always: true }));
    else window.localStorage.removeItem(DOCK_PREFERENCE_KEY);
  } catch {
    // Blocked storage keeps the answer for this page only.
  }
  notify();
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  // The answer given in another tab holds here too.
  const onStorage = (event: StorageEvent) => {
    if (event.key !== DOCK_PREFERENCE_KEY && event.key !== null) return;
    always = readStored();
    listener();
  };
  window.addEventListener("storage", onStorage);
  return () => {
    listeners.delete(listener);
    window.removeEventListener("storage", onStorage);
  };
}

export function useDockAlways(): boolean {
  return useSyncExternalStore(subscribe, dockAlways, () => false);
}
