/**
 * How hard a new password is to guess, on the three bars of the meter: none
 * lit below the minimum length, then weak, good and strong.
 *
 * Length counts most: a few ordinary words beat one short word dressed up in
 * symbols, and the meter says so. Variety helps a password that is long
 * already. A password that holds a word every list of leaked passwords
 * starts with, or the person's own login, stays weak however long it is.
 * The server enforces only the minimum; this is advice, not a rule.
 */

export const MIN_PASSWORD_LENGTH = 8;

export type PasswordScore = 0 | 1 | 2 | 3;

export interface PasswordStrength {
  score: PasswordScore;
  /**
   * Why the score is not higher: `short` below the minimum, `guessable` for a
   * common word, the login or a run of one character, `length` when only more
   * of it would help. Null for an empty password and for the top score.
   */
  hint: "short" | "guessable" | "length" | null;
}

/** Words that open every list of leaked passwords, after `fold`. */
const GUESSABLE = ["password", "haslo", "qwerty", "12345", "zaq1", "admin", "letmein", "stellarr"];

/** Lower case without diacritics, so the Polish word for password is found with or without them. */
function fold(text: string): string {
  return (
    text
      .toLowerCase()
      // The Polish l with a stroke is a letter of its own, not an l plus a
      // mark, so NFD leaves it.
      .replace(/\u0142/g, "l")
      .normalize("NFD")
      .replace(/[\u0300-\u036f]/g, "")
  );
}

/** Lower and upper letters, digits, other characters and spaces: how many kinds it mixes. */
function kinds(password: string): number {
  return [/[a-z]/, /[A-Z]/, /[0-9]/, /[^A-Za-z0-9\s]/, /\s/].filter((kind) => kind.test(password)).length;
}

export function passwordStrength(password: string, username = ""): PasswordStrength {
  if (password.length === 0) return { score: 0, hint: null };
  if (password.length < MIN_PASSWORD_LENGTH) return { score: 0, hint: "short" };

  const folded = fold(password);
  const login = fold(username.trim());
  const guessable =
    GUESSABLE.some((word) => folded.includes(word)) ||
    (login.length >= 3 && folded.includes(login)) ||
    new Set(folded).size <= 3;
  if (guessable) return { score: 1, hint: "guessable" };

  const mixed = kinds(password) >= 3;
  if (password.length >= 16 || (password.length >= 13 && mixed)) return { score: 3, hint: null };
  if (password.length >= 12 || (password.length >= 10 && mixed)) return { score: 2, hint: null };
  return { score: 1, hint: "length" };
}
