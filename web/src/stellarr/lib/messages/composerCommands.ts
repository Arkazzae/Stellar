/**
 * Finding a composer command from what was typed after the slash.
 *
 * A command answers to two names: the one in the reader's language and the
 * English one, which works in every language so that a habit survives a
 * change of language. Both are compared without case and without diacritics,
 * because Polish is typed both ways depending on the keyboard.
 */

/** The two names a command answers to, without the slash. */
export interface CommandNames {
  name: string;
  alias: string;
}

/** Which name matched, and the part of it to mark. */
export interface CommandMatch {
  via: "name" | "alias";
  /** UTF-16 offsets into that name; equal for an empty query. */
  from: number;
  to: number;
  /**
   * Lower is better. A name that starts with the query, then one that holds
   * it, then one whose letters follow it in order from the first; at each
   * step the reader's own name before the English one.
   */
  rank: number;
}

/** The command word being typed: a slash at the very start of the draft, with the caret on its word. */
export interface SlashQuery {
  /** The word after the slash, all of it, not only up to the caret. */
  query: string;
  /** Where the word ends; the draft after it is the command's surroundings. */
  end: number;
}

interface Folded {
  text: string;
  /** For each unit of `text`, where the letter it came from starts and ends. */
  starts: number[];
  ends: number[];
}

/** The Polish l with a stroke is a letter of its own, not an l plus a mark, so NFD leaves it. */
const STROKED_L = /\u0142/g;
const MARKS = /\p{M}/gu;

function foldLetter(letter: string): string {
  return letter.toLowerCase().replace(STROKED_L, "l").normalize("NFD").replace(MARKS, "");
}

/** Folds letter by letter, so a match in the folded text maps back to the name as written. */
function fold(text: string): Folded {
  const folded: Folded = { text: "", starts: [], ends: [] };
  let at = 0;
  for (const letter of text) {
    const plain = foldLetter(letter);
    for (let unit = 0; unit < plain.length; unit += 1) {
      folded.starts.push(at);
      folded.ends.push(at + letter.length);
    }
    folded.text += plain;
    at += letter.length;
  }
  return folded;
}

export function foldCommandText(text: string): string {
  return fold(text).text;
}

function inOrder(query: string, text: string): boolean {
  let from = 0;
  for (const unit of query) {
    const found = text.indexOf(unit, from);
    if (found === -1) return false;
    from = found + unit.length;
  }
  return true;
}

/** How `name` answers a folded query: its tier and the folded range to mark. */
function answer(query: string, name: string): { tier: number; from: number; to: number } | null {
  const folded = fold(name);
  const mark = (tier: number, from: number, to: number) =>
    from === to
      ? { tier, from: 0, to: 0 }
      : { tier, from: folded.starts[from] ?? 0, to: folded.ends[to - 1] ?? name.length };
  if (folded.text.startsWith(query)) return mark(0, 0, query.length);
  const inside = folded.text.indexOf(query);
  if (inside > 0) return mark(1, inside, inside + query.length);
  // The letters in order from the first, as "za" finds "zbanuj". Only the run
  // that agrees from the start is marked: the scattered letters after it say
  // less than the beginning does.
  if (folded.text[0] === query[0] && inOrder(query, folded.text)) {
    let common = 0;
    while (common < query.length && folded.text[common] === query[common]) common += 1;
    return mark(2, 0, common);
  }
  return null;
}

/** Whether, and how well, a command answers what was typed after the slash. */
export function matchCommand(query: string, names: CommandNames): CommandMatch | null {
  const needle = foldCommandText(query);
  const byName = answer(needle, names.name);
  // An English reader's name is the alias itself; asking twice adds nothing.
  const byAlias =
    foldCommandText(names.alias) === foldCommandText(names.name) ? null : answer(needle, names.alias);
  if (byName !== null && (byAlias === null || byName.tier <= byAlias.tier)) {
    return { via: "name", from: byName.from, to: byName.to, rank: byName.tier * 2 };
  }
  if (byAlias !== null) {
    return { via: "alias", from: byAlias.from, to: byAlias.to, rank: byAlias.tier * 2 + 1 };
  }
  return null;
}

/** True when the query is one of the names exactly, as folding sees them. */
export function namesCommand(query: string, names: CommandNames): boolean {
  const needle = foldCommandText(query);
  return needle === foldCommandText(names.name) || needle === foldCommandText(names.alias);
}

/**
 * The command word under the caret. Only a slash that opens the draft starts
 * one: a slash later in a sentence is a slash, and a caret beyond the first
 * word is writing the message, not the command.
 */
export function slashQuery(text: string, caret: number): SlashQuery | null {
  if (!text.startsWith("/")) return null;
  const space = text.search(/\s/);
  const end = space === -1 ? text.length : space;
  if (caret < 1 || caret > end) return null;
  return { query: text.slice(1, end), end };
}
