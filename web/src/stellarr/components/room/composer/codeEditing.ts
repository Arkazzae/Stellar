/**
 * Indenting in the composer's code mode, as a replacement of one range of the
 * draft. A range rather than a whole new draft, so the field can apply it as
 * typing, which keeps it on the browser's own undo history.
 */

export interface RangeEdit {
  /** The range of the draft that is replaced. */
  from: number;
  to: number;
  insert: string;
  /** Where the selection sits once the range is replaced. */
  selectionStart: number;
  selectionEnd: number;
}

const INDENT = "  ";

/** How much of a line's leading whitespace one outdent takes: a tab, or up to two spaces. */
function outdentWidth(line: string): number {
  if (line.startsWith("\t")) return 1;
  if (line.startsWith(INDENT)) return INDENT.length;
  return line.startsWith(" ") ? 1 : 0;
}

/**
 * Tab and Shift+Tab over the selection `start`..`end` of `text`.
 *
 * Without a selection Tab puts two spaces at the caret, as a key would. With
 * one, every line it touches moves by two spaces, except a blank line, which
 * would only gain trailing spaces; a selection that ends at the very start of
 * a line leaves that line alone. Shift+Tab takes a tab or up to two spaces
 * off the front of each touched line, the caret's own included. Null when
 * nothing would change.
 */
export function indentCode(
  text: string,
  start: number,
  end: number,
  outdent: boolean,
): RangeEdit | null {
  if (start === end && !outdent) {
    return {
      from: start,
      to: end,
      insert: INDENT,
      selectionStart: start + INDENT.length,
      selectionEnd: start + INDENT.length,
    };
  }

  const from = text.lastIndexOf("\n", start - 1) + 1;
  const last = end > start && text[end - 1] === "\n" ? end - 1 : end;
  const nextBreak = text.indexOf("\n", last);
  const to = nextBreak === -1 ? text.length : nextBreak;

  // Each line's change, as [where it starts in the draft, characters added (+)
  // or taken (-)].
  const changes: [number, number][] = [];
  let at = from;
  const lines = text.slice(from, to).split("\n");
  const changed = lines.map((line) => {
    const lineStart = at;
    at += line.length + 1;
    if (outdent) {
      const width = outdentWidth(line);
      if (width > 0) changes.push([lineStart, -width]);
      return line.slice(width);
    }
    if (line.trim().length === 0 && lines.length > 1) return line;
    changes.push([lineStart, INDENT.length]);
    return `${INDENT}${line}`;
  });
  if (changes.length === 0) return null;

  /** Where an offset of the old draft lands in the new one. */
  const map = (offset: number, isStart: boolean) =>
    changes.reduce((moved, [lineStart, delta]) => {
      if (delta > 0) {
        // A selection that starts at the very front of a line keeps covering
        // the indent added there.
        return offset > lineStart || (offset === lineStart && !isStart) ? moved + delta : moved;
      }
      return moved - Math.min(-delta, Math.max(0, offset - lineStart));
    }, offset);

  return {
    from,
    to,
    insert: changed.join("\n"),
    selectionStart: map(start, true),
    selectionEnd: map(end, false),
  };
}
