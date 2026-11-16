/**
 * The `customer_id` this browser sends GIPHY.
 *
 * GIPHY asks for one identifier per person, the same on every request. It is
 * made up here at random and kept in this browser, so it says nothing about
 * the account and the server never learns it. Clearing the site's data makes
 * a new one.
 */
export const GIPHY_CUSTOMER_ID_KEY = "stellarr.giphy.customer";

const SHAPE = /^[0-9a-f]{32}$/;

/** The id this page made, which holds for the page when storage refuses it. */
let made: string | null = null;

function randomId(): string {
  // getRandomValues rather than randomUUID, which browsers offer only on
  // secure origins, and an instance may be reached over plain HTTP at home.
  const bytes = crypto.getRandomValues(new Uint8Array(16));
  return Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("");
}

/**
 * This browser's id, made on first use. It is read from storage every time,
 * so two tabs that both made one settle on whichever was stored last.
 */
export function giphyCustomerId(): string {
  try {
    const stored = window.localStorage.getItem(GIPHY_CUSTOMER_ID_KEY);
    if (stored !== null && SHAPE.test(stored)) return stored;
  } catch {
    // Storage is refused; the page keeps the id it made.
  }
  made ??= randomId();
  try {
    window.localStorage.setItem(GIPHY_CUSTOMER_ID_KEY, made);
  } catch {
    // As above.
  }
  return made;
}
