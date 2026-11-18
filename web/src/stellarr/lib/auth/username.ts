/** The login rules the server checks, so the form can say them before it does. */
export const USERNAME_MIN_LENGTH = 3;
export const USERNAME_MAX_LENGTH = 32;

/**
 * A login made from a display name, for the form to fill in until the
 * person types one of their own: "Ola Nowak" becomes "ola.nowak", and a
 * Polish name loses its diacritics on the way.
 *
 * The server takes letters of any alphabet, but a login is typed to mention
 * someone, so the suggestion keeps to plain ASCII: diacritics go, spaces
 * become dots, and anything else that is not a letter, a digit, a dot, a
 * hyphen or an underscore is left out. A run of separators is one, and none
 * stands at either end.
 */
export function suggestUsername(displayName: string): string {
  return displayName
    .toLowerCase()
    // The Polish l with a stroke has no decomposition for NFD to strip.
    .replace(/\u0142/g, "l")
    .normalize("NFD")
    .replace(/[\u0300-\u036f]/g, "")
    .trim()
    .replace(/\s+/g, ".")
    .replace(/[^a-z0-9._-]/g, "")
    .replace(/([._-])[._-]+/g, "$1")
    .replace(/^[._-]+/, "")
    .slice(0, USERNAME_MAX_LENGTH)
    .replace(/[._-]+$/, "");
}

/** Whether the server will take `username` as a login: 3 to 32 letters, digits, ".", "-" or "_". */
export function isValidUsername(username: string): boolean {
  return (
    username.length >= USERNAME_MIN_LENGTH &&
    username.length <= USERNAME_MAX_LENGTH &&
    /^[\p{L}\p{N}_.-]+$/u.test(username)
  );
}
