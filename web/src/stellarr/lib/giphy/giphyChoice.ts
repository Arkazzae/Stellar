/**
 * Whether this browser may contact GIPHY: the person's own switch, on by
 * default, under Privacy in their settings.
 *
 * Off, nothing here calls GIPHY. The composer has no GIF button, and a GIF in
 * a message is a link to its page on GIPHY with a button that loads that one
 * GIF. The switch is kept in this browser and follows the account in the
 * privacy section of the client's settings, through `ClientSettingsSync`.
 *
 * On is not enough by itself. This browser's copy may be older than the
 * account's: the person may have turned GIPHY off on another device. So until
 * the account has answered in this page, or could not be asked, the switch
 * reads as waiting and GIPHY is not called. Off never waits.
 */

/** Where this browser keeps the switch, as `{"enabled":false}`; absent means on. */
export const GIPHY_CHOICE_KEY = "stellarr.giphy";

/** On for a person who never touched it. */
export const GIPHY_DEFAULT_ENABLED = true;

/** What the switch allows right now. */
export type GiphyAccess = "on" | "off" | "waiting";

function readStored(): boolean {
  try {
    const raw = window.localStorage.getItem(GIPHY_CHOICE_KEY);
    if (raw === null) return GIPHY_DEFAULT_ENABLED;
    const enabled = (JSON.parse(raw) as { enabled?: unknown } | null)?.enabled;
    return typeof enabled === "boolean" ? enabled : GIPHY_DEFAULT_ENABLED;
  } catch {
    return GIPHY_DEFAULT_ENABLED;
  }
}

/** A store of its own; the app uses `giphyChoice`, and tests make fresh ones. */
export function createGiphyChoice() {
  const listeners = new Set<() => void>();
  let enabled: boolean | undefined;
  let accountAnswered = false;

  const notify = () => {
    for (const listener of listeners) listener();
  };

  // Turning GIPHY off in one tab stops the others at once.
  const onStorage = (event: StorageEvent) => {
    if (event.key !== GIPHY_CHOICE_KEY && event.key !== null) return;
    enabled = readStored();
    notify();
  };

  const isEnabled = () => (enabled ??= readStored());

  return {
    /** The switch's position, stored or default, whatever the account says later. */
    isEnabled,

    /** What may be sent to GIPHY now. */
    access(): GiphyAccess {
      if (!isEnabled()) return "off";
      return accountAnswered ? "on" : "waiting";
    },

    subscribe(listener: () => void): () => void {
      if (listeners.size === 0) window.addEventListener("storage", onStorage);
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
        if (listeners.size === 0) window.removeEventListener("storage", onStorage);
      };
    },

    /** Moves the switch, by the person's hand or from the account's copy. */
    set(next: boolean): void {
      if (isEnabled() === next) return;
      enabled = next;
      try {
        window.localStorage.setItem(GIPHY_CHOICE_KEY, JSON.stringify({ enabled: next }));
      } catch {
        // Without storage the switch holds for this page.
      }
      notify();
    },

    /** A sign-in starts: the account's copy is about to be read. */
    awaitAccount(): void {
      if (!accountAnswered) return;
      accountAnswered = false;
      notify();
    },

    /** The account's copy is applied, or it could not be read and this browser's stands. */
    accountRead(): void {
      if (accountAnswered) return;
      accountAnswered = true;
      notify();
    },
  };
}

export type GiphyChoice = ReturnType<typeof createGiphyChoice>;

/** The page's one switch. */
export const giphyChoice: GiphyChoice = createGiphyChoice();
