/**
 * The character rules the message grammar and the composer share.
 *
 * Both decide whether a delimiter run may open or close around the words next
 * to it, so the Unicode boundaries and CommonMark's flanking test live here
 * once. The parser reads whole bodies; the composer asks the same questions
 * about a marker it is about to insert.
 */

export type DelimiterChar = "*" | "_" | "~" | "|";

const WHITESPACE = /^\s$/u;
/** CommonMark counts Unicode punctuation and symbols, so `*🎉*` is emphasis. */
const PUNCTUATION = /^[\p{P}\p{S}]$/u;

/** The code point that ends just before `index`, or "" at the start. */
export function charBefore(source: string, index: number): string {
  if (index <= 0) return "";
  const low = source.charCodeAt(index - 1);
  const high = source.charCodeAt(index - 2);
  const pair = low >= 0xdc00 && low <= 0xdfff && high >= 0xd800 && high <= 0xdbff;
  return source.slice(pair ? index - 2 : index - 1, index);
}

/** The code point that starts at `index`, or "" at the end. */
export function charAt(source: string, index: number): string {
  const code = source.codePointAt(index);
  return code === undefined ? "" : String.fromCodePoint(code);
}

/** Whitespace, or a control character, by UTF-16 unit: no address contains either. */
export function isBlankUnit(code: number): boolean {
  return code <= 0x20 || code === 0x7f || (code > 0x7f && WHITESPACE.test(String.fromCharCode(code)));
}

/** The characters JavaScript's `\s` matches. */
export function isWhitespace(char: string): boolean {
  return WHITESPACE.test(char);
}

/** The start and end of the body count as whitespace, as a line's ends do in CommonMark. */
export function isSpace(char: string): boolean {
  return char === "" || isWhitespace(char);
}

export function isPunctuation(char: string): boolean {
  return PUNCTUATION.test(char);
}

/** How many of `char` start at `from`. */
export function runLength(source: string, from: number, char: string): number {
  let end = from;
  while (source[end] === char) end += 1;
  return end - from;
}

/** How many of `char` end just before `index`. */
export function runBefore(source: string, index: number, char: string): number {
  let start = index;
  while (start > 0 && source[start - 1] === char) start -= 1;
  return index - start;
}

/** How many of `char` start at `index`. */
export function runAfter(source: string, index: number, char: string): number {
  let end = index;
  while (end < source.length && source[end] === char) end += 1;
  return end - index;
}

/** Whether the characters around a run are left- and right-flanking. */
function flanking(before: string, after: string): { left: boolean; right: boolean } {
  const left = !isSpace(after) && (!isPunctuation(after) || isSpace(before) || isPunctuation(before));
  const right = !isSpace(before) && (!isPunctuation(before) || isSpace(after) || isPunctuation(after));
  return { left, right };
}

/**
 * Whether a run of `length` characters may open, by CommonMark's flanking
 * rules. `~~` follows `*`, `_` must not open inside a word, and `||` needs no
 * flanking at all.
 */
export function canOpenDelimiter(
  character: DelimiterChar,
  before: string,
  after: string,
  length: number,
): boolean {
  if (character === "|") return length >= 2;
  const { left, right } = flanking(before, after);
  if (character === "~") return left && length === 2;
  if (character === "_") return left && (!right || isPunctuation(before));
  return left;
}

/** The mirror image of `canOpenDelimiter`, for a run that may close. */
export function canCloseDelimiter(
  character: DelimiterChar,
  before: string,
  after: string,
  length: number,
): boolean {
  if (character === "|") return length >= 2;
  const { left, right } = flanking(before, after);
  if (character === "~") return right && length === 2;
  if (character === "_") return right && (!left || isPunctuation(after));
  return right;
}

/** How the run of `character` at `index` may pair, as typed. */
export function delimiterRun(
  source: string,
  index: number,
  character: DelimiterChar,
  length: number,
): { canOpen: boolean; canClose: boolean } {
  const before = charBefore(source, index);
  const after = charAt(source, index + length);
  return {
    canOpen: canOpenDelimiter(character, before, after, length),
    canClose: canCloseDelimiter(character, before, after, length),
  };
}
