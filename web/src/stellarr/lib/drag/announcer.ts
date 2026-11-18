/**
 * What a drag says, kept outside React state: a new sentence re-renders the
 * live region that reads it and nothing around the list being sorted.
 */
export interface Announcer {
  say: (text: string) => void;
  read: () => string;
  subscribe: (listener: () => void) => () => void;
}

export function createAnnouncer(): Announcer {
  let text = "";
  const listeners = new Set<() => void>();
  return {
    say: (next) => {
      text = next;
      for (const listener of listeners) listener();
    },
    read: () => text,
    subscribe: (listener) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
  };
}
