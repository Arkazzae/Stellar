/**
 * The inline grammar of a message body, as a tree of spans.
 *
 * The grammar is small on purpose: `**strong**`, `*emphasis*` and `_emphasis_`
 * paired by CommonMark's delimiter-run rules, `~~strike~~`, `||spoiler||`,
 * `` `code` `` spans, backslash escapes and `[label](http…)` links. Nothing
 * else is markup: HTML, autolinks in angle brackets, images, references and
 * entities stay the characters that were typed.
 *
 * The body is read once from left to right into a list of spans, with a stack
 * of the delimiter runs that may still pair. A link is settled when its `]` is
 * read; emphasis is paired afterwards by CommonMark's "process emphasis"
 * procedure, which only relinks spans, so every step is linear in the body.
 */

import {
  delimiterRun,
  isBlankUnit,
  isWhitespace,
  runLength,
  type DelimiterChar,
} from "./inlineSyntax";

export type SpanStyle = "strong" | "emphasis" | "strike" | "spoiler";

export type Span =
  /** Literal text; the recognisers for URLs, mentions and emoji read it. */
  | { type: "text"; text: string }
  /**
   * Literal text the recognisers must not read: a character typed after a
   * backslash, or formatting nested too deep, shown as it was typed. `\@ada`
   * is the way to write a name without mentioning anybody.
   */
  | { type: "verbatim"; text: string }
  | { type: "code"; text: string }
  | { type: "link"; href: string; label: string }
  | { type: "styled"; style: SpanStyle; children: Span[] };

/* ------------------------------------------------------------- characters */

/** Only ASCII punctuation can be escaped, as in CommonMark. */
const ASCII_PUNCTUATION = /^[!-/:-@[-`{-~]$/;

/** The characters that may start a construct; everything between them is text. */
const SPECIAL = /[\\`*_~|[\]]/g;

/* ------------------------------------------------------------- span list */

/**
 * A span while the body is being read. Spans sit in a doubly linked list so
 * that pairing a delimiter can move everything between opener and closer into
 * a new span without copying it.
 */
interface Item {
  type: "text" | "verbatim" | "code" | "link" | "styled";
  /** The text, the code, or a link's label; for a delimiter run, its character. */
  text: string;
  /**
   * How many of `text` a delimiter run still holds. Kept as a count because a
   * long run is used up two characters at a time.
   */
  repeat?: number;
  href?: string;
  style?: SpanStyle;
  /** Where a styled span's source starts and ends, delimiters included. */
  start?: number;
  end?: number;
  /** A styled span's content. */
  first?: Item | null;
  last?: Item | null;
  prev: Item | null;
  next: Item | null;
}

function itemText(item: Item): string {
  return item.repeat === undefined ? item.text : item.text.repeat(item.repeat);
}

class ItemList {
  first: Item | null = null;
  last: Item | null = null;

  push(type: Item["type"], text: string): Item {
    const item: Item = { type, text, prev: this.last, next: null };
    if (this.last === null) this.first = item;
    else this.last.next = item;
    this.last = item;
    return item;
  }

  remove(item: Item): void {
    if (item.prev === null) this.first = item.next;
    else item.prev.next = item.next;
    if (item.next === null) this.last = item.prev;
    else item.next.prev = item.prev;
  }

  /** Cuts out everything after `item` and returns the first of it. */
  cutAfter(item: Item): Item | null {
    const cut = item.next;
    item.next = null;
    this.last = item;
    if (cut !== null) cut.prev = null;
    return cut;
  }
}

/** A link's label is plain: whatever formatting it carried is read as its text. */
function flatten(first: Item | null): string {
  let text = "";
  const resume: (Item | null)[] = [];
  let item = first;
  for (;;) {
    if (item === null) {
      if (resume.length === 0) return text;
      item = resume.pop() ?? null;
      continue;
    }
    if (item.type === "styled") {
      resume.push(item.next);
      item = item.first ?? null;
      continue;
    }
    text += itemText(item);
    item = item.next;
  }
}

/* ------------------------------------------------------------- delimiters */

/** A run of `*`, `_`, `~` or `|` that may still open or close a span. */
interface Delimiter {
  char: DelimiterChar;
  /** The text span holding the characters of the run not used yet. */
  item: Item;
  /** Where the run was typed; it never moves, so it orders the stack. */
  position: number;
  /** The run as typed, for the rule of three. */
  length: number;
  /** Characters not used yet, and where they are in the source. */
  count: number;
  start: number;
  end: number;
  canOpen: boolean;
  canClose: boolean;
  prev: Delimiter | null;
  next: Delimiter | null;
}

class DelimiterStack {
  bottom: Delimiter | null = null;
  top: Delimiter | null = null;

  push(delimiter: Delimiter): void {
    delimiter.prev = this.top;
    if (this.top === null) this.bottom = delimiter;
    else this.top.next = delimiter;
    this.top = delimiter;
  }

  remove(delimiter: Delimiter): void {
    if (delimiter.prev === null) this.bottom = delimiter.next;
    else delimiter.prev.next = delimiter.next;
    if (delimiter.next === null) this.top = delimiter.prev;
    else delimiter.next.prev = delimiter.prev;
  }

  /** Drops every delimiter above `floor`, or all of them. */
  clearAbove(floor: Delimiter | null): void {
    if (floor === null) {
      this.bottom = null;
      this.top = null;
    } else {
      floor.next = null;
      this.top = floor;
    }
  }
}

/* ------------------------------------------------------------ the reader */

const SCHEMES = ["http://", "https://"];

/** A `[` that a later `]` may still close. */
interface Bracket {
  item: Item;
  position: number;
  /** `![`: images are not supported, but the bracket must still pair. */
  image: boolean;
  /** The top of the delimiter stack when the bracket was read. */
  floor: Delimiter | null;
  previous: Bracket | null;
}

/** A run that cannot pair again: used up, or a spoiler run down to one `|`. */
function spent(delimiter: Delimiter): boolean {
  return delimiter.count === 0 || (delimiter.char === "|" && delimiter.count < 2);
}

class InlineReader {
  private readonly list = new ItemList();
  private readonly delimiters = new DelimiterStack();
  private brackets: Bracket | null = null;
  /** Brackets read before this offset can no longer start a link: no links in links. */
  private inactiveBefore = -1;
  private escapedUpTo = -1;

  private backtickRuns: Map<number, { starts: number[]; next: number }> | undefined;
  private closingParen: Int32Array | undefined;
  private visibleBefore: Int32Array | undefined;

  private readonly source: string;
  private readonly plain: readonly (readonly [number, number])[];

  constructor(source: string, plain: readonly (readonly [number, number])[]) {
    this.source = source;
    this.plain = plain;
  }

  read(): ItemList {
    const { source, list, plain } = this;
    let index = 0;
    let next = 0;
    let found: RegExpExecArray | null | undefined;
    while (index < source.length) {
      // A stretch that started inside something already read is no longer plain.
      while ((plain[next]?.[0] ?? Infinity) < index) next += 1;
      // Keep the next marker while skipping URLs. Searching the remaining
      // suffix again for each address would be quadratic on a list of URLs
      // with no markers anywhere in it.
      if (found === undefined || (found !== null && found.index < index)) {
        SPECIAL.lastIndex = index;
        found = SPECIAL.exec(source);
      }
      const at = found === null ? source.length : found.index;
      const stretch = plain[next];
      if (stretch !== undefined && stretch[0] < at) {
        list.push("text", source.slice(index, stretch[1]));
        index = stretch[1];
        continue;
      }
      if (at > index) list.push("text", source.slice(index, at));
      if (at >= source.length) break;
      index = this.special(at);
    }
    this.processEmphasis(null);
    return list;
  }

  /** Reads the construct that starts at `index` and returns where reading resumes. */
  private special(index: number): number {
    const { source, list } = this;
    const char = source[index] ?? "";
    switch (char) {
      case "\\": {
        const escaped = source[index + 1] ?? "";
        if (!ASCII_PUNCTUATION.test(escaped)) {
          list.push("text", "\\");
          return index + 1;
        }
        list.push("verbatim", escaped);
        this.escapedUpTo = index + 2;
        return index + 2;
      }
      case "`":
        return this.codeSpan(index);
      case "[":
        this.brackets = {
          item: list.push("text", "["),
          position: index,
          image: source[index - 1] === "!" && this.escapedUpTo !== index,
          floor: this.delimiters.top,
          previous: this.brackets,
        };
        return index + 1;
      case "]":
        return this.closeBracket(index);
      default: {
        const delimiter = char as DelimiterChar;
        const length = runLength(source, index, delimiter);
        const item = list.push("text", delimiter);
        item.repeat = length;
        const { canOpen, canClose } = delimiterRun(source, index, delimiter, length);
        if (canOpen || canClose) {
          this.delimiters.push({
            char: delimiter,
            item,
            position: index,
            length,
            count: length,
            start: index,
            end: index + length,
            canOpen,
            canClose,
            prev: null,
            next: null,
          });
        }
        return index + length;
      }
    }
  }

  /**
   * A run of backticks opens a code span that the next run of the same length
   * closes; without one, the run is literal. Inside, everything is literal,
   * backslashes included, and one space is stripped from each end when both
   * ends have one, so `` `` `a` `` `` can show backticks.
   */
  private codeSpan(index: number): number {
    const { source, list } = this;
    const length = runLength(source, index, "`");
    const close = this.nextBacktickRun(length, index + length);
    if (close < 0) {
      list.push("text", "`".repeat(length));
      return index + length;
    }
    let code = source.slice(index + length, close);
    if (code.length >= 2 && code.startsWith(" ") && code.endsWith(" ") && /[^ ]/.test(code)) {
      code = code.slice(1, -1);
    }
    list.push("code", code);
    return close + length;
  }

  /**
   * Where the next run of exactly `length` backticks starts at or after
   * `from`, or -1. The runs are listed once per body and each length's cursor
   * only moves forward, since reading does.
   */
  private nextBacktickRun(length: number, from: number): number {
    if (this.backtickRuns === undefined) {
      this.backtickRuns = new Map();
      const { source } = this;
      for (let index = source.indexOf("`"); index >= 0; ) {
        const run = runLength(source, index, "`");
        const entry = this.backtickRuns.get(run) ?? { starts: [], next: 0 };
        entry.starts.push(index);
        this.backtickRuns.set(run, entry);
        index = source.indexOf("`", index + run);
      }
    }
    const entry = this.backtickRuns.get(length);
    if (entry === undefined) return -1;
    while (entry.next < entry.starts.length && (entry.starts[entry.next] ?? 0) < from) entry.next += 1;
    return entry.starts[entry.next] ?? -1;
  }

  /**
   * A `]` pairs with the nearest `[` still open. With `(http://…)` or
   * `(https://…)` right after it, the two make a link; otherwise both are
   * literal. The label's own emphasis is paired first, within the brackets.
   */
  private closeBracket(index: number): number {
    const { list } = this;
    const opener = this.brackets;
    if (opener === null) {
      list.push("text", "]");
      return index + 1;
    }
    this.brackets = opener.previous;

    const target = opener.image || opener.position < this.inactiveBefore ? undefined : this.linkTarget(index + 1);
    if (target === undefined) {
      list.push("text", "]");
      return index + 1;
    }

    this.processEmphasis(opener.floor);
    const label = flatten(list.cutAfter(opener.item));
    opener.item.type = "link";
    opener.item.href = target.href;
    // A link nobody can see is no use: an empty label shows the address.
    opener.item.text = label.trim() === "" ? target.href : label;
    this.inactiveBefore = opener.position;
    return target.end;
  }

  /**
   * `(http://…)` at `index`: an http(s) address up to the `)` that balances
   * its `(`, with no whitespace or control character in it. Any other scheme
   * or shape is not a link.
   */
  private linkTarget(index: number): { href: string; end: number } | undefined {
    const { source } = this;
    if (source[index] !== "(") return undefined;
    const scheme = SCHEMES.find(
      (candidate) => source.slice(index + 1, index + 1 + candidate.length).toLowerCase() === candidate,
    );
    if (scheme === undefined) return undefined;

    const close = this.matchingParen(index);
    if (close <= index + 1 + scheme.length) return undefined;
    return { href: source.slice(index + 1, close), end: close + 1 };
  }

  /**
   * The `)` that balances the `(` at `open`, or -1. Worked out for the whole
   * body in one pass the first time a link is tried; the count restarts at
   * whitespace, which no address contains.
   */
  private matchingParen(open: number): number {
    if (this.closingParen === undefined) {
      const { source } = this;
      const closing = new Int32Array(source.length).fill(-1);
      const opened: number[] = [];
      for (let index = 0; index < source.length; index++) {
        const code = source.charCodeAt(index);
        if (code === 0x28) opened.push(index);
        else if (code === 0x29) {
          const from = opened.pop();
          if (from !== undefined) closing[from] = index;
        } else if (isBlankUnit(code)) {
          opened.length = 0;
        }
      }
      this.closingParen = closing;
    }
    return this.closingParen[open] ?? -1;
  }

  /** Whether anything but whitespace was typed between two offsets. */
  private visibleBetween(from: number, to: number): boolean {
    if (this.visibleBefore === undefined) {
      const { source } = this;
      const counts = new Int32Array(source.length + 1);
      for (let index = 0; index < source.length; index++) {
        const code = source.charCodeAt(index);
        const blank = code === 0x20 || (code >= 0x09 && code <= 0x0d) ||
          (code > 0x7f && isWhitespace(String.fromCharCode(code)));
        counts[index + 1] = (counts[index] ?? 0) + (blank ? 0 : 1);
      }
      this.visibleBefore = counts;
    }
    return (this.visibleBefore[to] ?? 0) - (this.visibleBefore[from] ?? 0) > 0;
  }

  /** Whether `opener` may pair with `closer`, both of the same character. */
  private pairs(opener: Delimiter, closer: Delimiter): boolean {
    if (closer.char === "|") return this.visibleBetween(opener.end, closer.start);
    if (closer.char === "~") return true;
    // The rule of three: when either run could both open and close, their
    // lengths must not add up to a multiple of three unless both are
    // multiples of three, so `*foo**bar*` is one emphasis, not two broken ones.
    return !(
      (opener.canClose || closer.canOpen) &&
      (opener.length + closer.length) % 3 === 0 &&
      !(opener.length % 3 === 0 && closer.length % 3 === 0)
    );
  }

  /**
   * CommonMark's "process emphasis": each closer, from the left, pairs with
   * the nearest opener of its character below it, and everything between them
   * becomes the new span's content. A spoiler takes only the nearest `||`.
   *
   * The openers-bottom table keeps this linear: once a closer finds nothing,
   * no later closer of the same kind looks below it again. Kinds differ by
   * character and, for `*` and `_`, by what the rule of three depends on.
   */
  private processEmphasis(floor: Delimiter | null): void {
    const { delimiters, list } = this;
    const searchedTo = new Map<string, number>();
    const floorPosition = floor?.position ?? -1;
    let closer = floor === null ? delimiters.bottom : floor.next;

    while (closer !== null) {
      if (!closer.canClose) {
        closer = closer.next;
        continue;
      }
      const kind =
        closer.char === "*" || closer.char === "_"
          ? `${closer.char}${closer.canOpen ? 1 : 0}${closer.length % 3}`
          : closer.char;
      const lowest = Math.max(floorPosition, searchedTo.get(kind) ?? -1);

      let opener = closer.prev;
      while (opener !== null && opener.position > lowest) {
        if (opener.char === closer.char && opener.canOpen) {
          if (this.pairs(opener, closer)) break;
          if (closer.char === "|") {
            opener = null;
            break;
          }
        }
        opener = opener.prev;
      }
      if (opener !== null && opener.position <= lowest) opener = null;

      if (opener === null) {
        searchedTo.set(kind, closer.prev?.position ?? -1);
        const next = closer.next;
        if (!closer.canOpen) delimiters.remove(closer);
        closer = next;
        continue;
      }

      const used = (closer.char === "*" || closer.char === "_") && (opener.count < 2 || closer.count < 2) ? 1 : 2;
      const style: SpanStyle =
        closer.char === "~" ? "strike" : closer.char === "|" ? "spoiler" : used === 2 ? "strong" : "emphasis";

      opener.count -= used;
      opener.end -= used;
      opener.item.repeat = opener.count;
      const spanEnd = closer.start + used;
      closer.count -= used;
      closer.start += used;
      closer.item.repeat = closer.count;

      // Everything between the two runs moves into the new span, and the
      // delimiters in it can no longer pair with anything outside.
      const content = opener.item.next;
      const styled: Item = {
        type: "styled",
        text: "",
        style,
        start: opener.end,
        end: spanEnd,
        first: content === closer.item ? null : content,
        last: content === closer.item ? null : closer.item.prev,
        prev: opener.item,
        next: closer.item,
      };
      if (styled.first) styled.first.prev = null;
      if (styled.last) styled.last.next = null;
      opener.item.next = styled;
      closer.item.prev = styled;
      opener.next = closer;
      closer.prev = opener;

      if (opener.count === 0) list.remove(opener.item);
      if (spent(opener)) delimiters.remove(opener);
      if (spent(closer)) {
        const next = closer.next;
        if (closer.count === 0) list.remove(closer.item);
        delimiters.remove(closer);
        closer = next;
      }
    }
    delimiters.clearAbove(floor);
  }
}

/* ------------------------------------------------------------- the tree */

function toSpans(first: Item | null, source: string, depth: number, maxDepth: number): Span[] {
  const spans: Span[] = [];
  for (let item = first; item !== null; item = item.next) {
    switch (item.type) {
      case "styled":
        spans.push(
          depth >= maxDepth
            ? { type: "verbatim", text: source.slice(item.start, item.end) }
            : {
                type: "styled",
                style: item.style ?? "emphasis",
                children: toSpans(item.first ?? null, source, depth + 1, maxDepth),
              },
        );
        break;
      case "link":
        spans.push({ type: "link", href: item.href ?? "", label: item.text });
        break;
      default:
        spans.push({ type: item.type, text: itemText(item) });
    }
  }
  return spans;
}

/**
 * The spans of `source`. Formatting nested more than `maxDepth` deep is shown
 * as it was typed. `plain` lists stretches, as ordered offsets, that are read
 * as text whatever they contain: the bare URLs.
 */
export function readInline(
  source: string,
  maxDepth: number,
  plain: readonly (readonly [start: number, end: number])[] = [],
): Span[] {
  return toSpans(new InlineReader(source, plain).read().first, source, 0, maxDepth);
}
