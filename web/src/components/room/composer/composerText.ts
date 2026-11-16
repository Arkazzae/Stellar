import { codeSource } from "@/lib/messages/inline";
import {
  canCloseDelimiter,
  canOpenDelimiter,
  charAt,
  charBefore,
  isWhitespace,
  runAfter,
  runBefore,
  type DelimiterChar,
} from "@/lib/messages/inlineSyntax";

/** A new draft and where the selection should sit in it. */
export interface TextEdit {
  text: string;
  selectionStart: number;
  selectionEnd: number;
}

/**
 * Puts an emoji at `at`, spaced from the words around it so the next thing
 * typed starts a new word instead of sticking to the glyph.
 */
export function insertEmojiAt(text: string, at: number, glyph: string): TextEdit {
  const before = text.slice(0, at);
  const after = text.slice(at);
  const lead = before.length > 0 && !/\s$/.test(before) ? " " : "";
  const trail = /^\s/.test(after) ? "" : " ";
  const inserted = `${lead}${glyph}${trail}`;
  const caret = before.length + inserted.length;
  return {
    text: `${before}${inserted}${after}`,
    selectionStart: caret,
    selectionEnd: caret,
  };
}

/** Every marker the formatting panel can produce, in the order it shows them. */
export const FORMAT_MARKERS = ["**", "*", "~~", "||", "`"] as const;

/** The formatting a button adds, and whether CommonMark's flanking rules apply. */
const FLANKING_MARKERS = new Set<string>(["*", "**", "~~"]);

/**
 * Whether a marker inserted at `index` may open a span, counting the run of
 * the same character already there. A strike run has to be exactly two, so an
 * adjacent tilde stops it from opening at all.
 */
function opensAt(text: string, index: number, character: string, length: number): boolean {
  const adjacent = runBefore(text, index, character);
  const before = charBefore(text, index - adjacent);
  const after = charAt(text, index);
  return canOpenDelimiter(character as DelimiterChar, before, after, adjacent + length);
}

/** Whether a marker inserted at `index` may close a span, counting the run after it. */
function closesAt(text: string, index: number, character: string, length: number): boolean {
  const adjacent = runAfter(text, index, character);
  const before = charBefore(text, index);
  const after = charAt(text, index + adjacent);
  return canCloseDelimiter(character as DelimiterChar, before, after, adjacent + length);
}

/**
 * The wrapper's own character, escaped where it would otherwise close the
 * wrapper. A character already escaped by the sender stays as it is, and a
 * trailing run of backslashes is made even so the closing marker is not the
 * one it escapes. The visible text is unchanged.
 */
function escapeDelimiter(text: string, character: string): string {
  let escaped = "";
  let backslashes = 0;
  for (const value of text) {
    if (value === "\\") {
      backslashes += 1;
    } else if (value === character) {
      if (backslashes % 2 === 0) escaped += "\\";
      backslashes = 0;
    } else {
      backslashes = 0;
    }
    escaped += value;
  }
  // An odd number at the end would escape the marker that follows the selection.
  return backslashes % 2 === 1 ? `${escaped}\\` : escaped;
}

/** The inverse of `escapeDelimiter`, for a run the composer wrapped itself. */
function unescapeDelimiter(text: string, character: string): string {
  return text.replaceAll(`\\${character}`, character);
}

/**
 * The code span's fence and padding around the selection, when the selection
 * is exactly the content of one. `null` when the neighbouring backticks do not
 * form one span.
 */
function codeSpanAround(
  before: string,
  after: string,
  selected: string,
): { prefix: number; text: string; selectionStart: number } | null {
  const bare = { left: /(`+)$/.exec(before), right: /^(`+)/.exec(after), pad: 0 };
  const padded = { left: /(`+) $/.exec(before), right: /^ (`+)/.exec(after), pad: 1 };
  for (const candidate of [bare, padded]) {
    if (
      candidate.left === null || candidate.right === null ||
      candidate.left[1]?.length !== candidate.right[1]?.length
    ) {
      continue;
    }
    const prefix = (candidate.left[1]?.length ?? 0) + candidate.pad;
    return {
      prefix,
      text: before.slice(0, before.length - prefix) + selected + after.slice(prefix),
      selectionStart: before.length - prefix,
    };
  }
  return null;
}

/** Padding `codeSource` puts inside the fence for a content that needs it. */
function codePadding(code: string): number {
  const spaced = code.startsWith(" ") && code.endsWith(" ") && /[^ ]/.test(code);
  return code.startsWith("`") || code.endsWith("`") || spaced ? 1 : 0;
}

/** Wraps or unwraps inline code, with a fence the content cannot close. */
function toggleCode(text: string, start: number, end: number): TextEdit {
  const before = text.slice(0, start);
  const after = text.slice(end);
  const selected = text.slice(start, end);
  const existing = codeSpanAround(before, after, selected);
  if (existing !== null) {
    return {
      text: existing.text,
      selectionStart: existing.selectionStart,
      selectionEnd: existing.selectionStart + selected.length,
    };
  }

  const source = codeSource(selected);
  const fence = (source.match(/^`+/) ?? [""])[0]?.length ?? 1;
  const selectionStart = before.length + fence + codePadding(selected);
  return {
    text: `${before}${source}${after}`,
    selectionStart,
    selectionEnd: selectionStart + selected.length,
  };
}

/**
 * Wraps the selection in a Markdown marker, or unwraps it when the marker is
 * already on both sides. With nothing selected, the caret lands inside a fresh
 * pair. The selection stays on the same words either way.
 *
 * Whitespace at either end stays outside the markers: a marker next to
 * whitespace never opens or closes a span. When the selected edge is
 * punctuation that cannot sit against a neighbouring word, the run widens to
 * the nearest word characters, because CommonMark has no way to format that
 * punctuation alone. The wrapper's own character is escaped where it would
 * close the new run early, so literal text such as `a**b` stays literal. The
 * characters keep their order and their visible text; only markup is added.
 */
export function toggleMarker(
  text: string,
  start: number,
  end: number,
  marker: string,
): TextEdit {
  const from = Math.max(0, Math.min(start, text.length));
  const to = Math.max(0, Math.min(end, text.length));
  const selectionStart = Math.min(from, to);
  const selectionEnd = Math.max(from, to);
  const before = text.slice(0, selectionStart);
  const selected = text.slice(selectionStart, selectionEnd);
  const after = text.slice(selectionEnd);
  if (marker.length === 0) {
    return { text, selectionStart, selectionEnd };
  }
  if (marker === "`") {
    return toggleCode(text, selectionStart, selectionEnd);
  }

  if (before.endsWith(marker) && after.startsWith(marker)) {
    const inner = unescapeDelimiter(selected, marker[0] ?? "");
    const unwrappedStart = selectionStart - marker.length;
    return {
      text: `${before.slice(0, -marker.length)}${inner}${after.slice(marker.length)}`,
      selectionStart: unwrappedStart,
      selectionEnd: unwrappedStart + inner.length,
    };
  }

  let coreStart = selectionStart;
  while (coreStart < selectionEnd && isWhitespace(text[coreStart] ?? "")) coreStart += 1;
  let coreEnd = selectionEnd;
  while (coreEnd > coreStart && isWhitespace(text[coreEnd - 1] ?? "")) coreEnd -= 1;

  // Nothing to format: leave an empty pair where the caret can type into it.
  if (coreStart === coreEnd) {
    return {
      text: `${before}${marker}${marker}${selected}${after}`,
      selectionStart: selectionStart + marker.length,
      selectionEnd: selectionStart + marker.length,
    };
  }

  if (FLANKING_MARKERS.has(marker)) {
    const character = marker[0] ?? "";
    // A marker before a word cannot be followed by punctuation, and the mirror
    // image at the close. A run of the marker's own character beside the
    // selection joins the new marker, so flanking is judged at the end of that
    // run. Widen to the neighbouring word rather than leave punctuation-only
    // markers that render as stars.
    while (!opensAt(text, coreStart, character, marker.length) && coreStart > 0) {
      coreStart -= Math.max(1, runBefore(text, coreStart, character));
    }
    while (!closesAt(text, coreEnd, character, marker.length) && coreEnd < text.length) {
      coreEnd += Math.max(1, runAfter(text, coreEnd, character));
    }
  }

  const core = text.slice(coreStart, coreEnd);
  const escaped = escapeDelimiter(core, marker[0] ?? "");
  const wrappedStart = coreStart + marker.length;
  return {
    text: `${text.slice(0, coreStart)}${marker}${escaped}${marker}${text.slice(coreEnd)}`,
    selectionStart: wrappedStart,
    selectionEnd: wrappedStart + escaped.length,
  };
}

/** The body of a message sent in code mode. */
export function fencedCode(language: string, source: string): string {
  return `\`\`\`${language}\n${source}\n\`\`\``;
}
