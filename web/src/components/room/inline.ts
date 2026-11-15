/**
 * Inline formatting of a message body.
 *
 * `Message.content` is whatever a stranger typed, so the transcript never sees a
 * string it may trust. A body is parsed into the nodes below and components
 * render nodes, never markup. This module is the only place that decides what
 * counts as formatting; anything it does not recognise stays literal text, which
 * React escapes on output.
 */

import { readInline, type Span } from "./inlineGrammar";

/** A run inside a text body. `text` is literal; React escapes it. */
export type InlineNode =
  | { kind: "text"; text: string }
  | { kind: "code"; text: string }
  | { kind: "strong" | "emphasis" | "strike" | "spoiler"; text: string; children?: InlineNode[] }
  | { kind: "link"; href: string; text: string }
  | {
      kind: "mention";
      text: string;
      userId: string;
      username: string;
      displayName: string;
      self: boolean;
    }
  /**
   * One of the server's own emoji, written `:code:` in the body.
   *
   * A node rather than a substitution back into the text, because it renders as
   * an `<img>` from the server and its URL comes from the server's own list,
   * never from anything the sender typed. `:not_an_emoji:` stays literal.
   */
  | { kind: "emoji"; shortcode: string; src: string }
  /**
   * The "(edytowano)" mark at the end of a message its author rewrote.
   *
   * A node rather than appended text, because nobody typed it: it must not be
   * selectable or searchable as part of the body, and typing the same characters
   * must not produce it.
   */
  | { kind: "edited"; at: string };

export interface MentionTarget {
  userId: string;
  username: string;
  displayName: string;
  self?: boolean;
}

export interface ParseInlineOptions {
  /**
   * The server's custom emoji, shortcode to image URL. The map is the
   * allow-list: a shortcode without an entry stays literal text.
   */
  emoji?: ReadonlyMap<string, string>;
  /** Lower-cased username to the member a mention resolves to. */
  mentions?: ReadonlyMap<string, MentionTarget>;
}

/* ---------------------------------------------------- literal text recognisers */

/** Only http(s), up to whitespace or a character that is never raw in a URL. */
const URL_RUN = /https?:\/\/[^\s<>"'`]+/g;

/** Punctuation that far more often ends the sentence than the URL. */
const TRAILING = new Set([".", ",", ";", ":", "!", "?", "…", "'", '"']);

/** The same, with the markers that may close formatting written around a URL. */
const TRAILING_IN_FORMATTING = new Set([...TRAILING, "*", "_", "~", "|"]);

function trimUrl(raw: string, trailing: ReadonlySet<string> = TRAILING): string {
  // Brackets still inside the URL, kept up to date as characters come off the
  // end, so a URL ending in many brackets is not recounted for each one.
  const inside = new Map<string, number>([["(", 0], [")", 0], ["[", 0], ["]", 0]]);
  for (const char of raw) if (inside.has(char)) inside.set(char, (inside.get(char) ?? 0) + 1);

  let end = raw.length;
  while (end > 0) {
    const last = raw[end - 1] ?? "";
    if (trailing.has(last)) {
      end -= 1;
      continue;
    }
    // A closing bracket belongs to the URL only if it was opened inside it:
    // "(see https://x/a)" keeps the paren out, "…/a_(b)" keeps it in.
    if (last === ")" || last === "]") {
      const open = last === ")" ? "(" : "[";
      const closes = inside.get(last) ?? 0;
      if (closes > (inside.get(open) ?? 0)) {
        inside.set(last, closes - 1);
        end -= 1;
        continue;
      }
    }
    break;
  }
  return raw.slice(0, end);
}

/** `@name`: usernames are letters, digits, `_`, `.` and `-`, 2 to 32 long. */
const MENTION_RUN = /@([\p{L}\p{N}_.-]{2,32})/gu;
const USERNAME_CHARACTER = /[\p{L}\p{N}_.-]/u;

/**
 * `:code:` is narrow on purpose (letters, digits, `_`, `-`): otherwise every
 * colon in a sentence is a candidate and "18:30 :" starts looking for an emoji
 * called "30".
 */
const SHORTCODE = /:([a-zA-Z0-9_-]{2,64}):/g;

interface Replacement {
  node: InlineNode;
  /** How much of the text the node stands for, from the start of the match. */
  length: number;
}

/**
 * Replaces the matches of `pattern` inside the text nodes with whatever
 * `replace` makes of them. A match it declines, and everything between matches,
 * stays text. Nodes that are not text pass through untouched.
 */
function replaceInText(
  nodes: readonly InlineNode[],
  pattern: RegExp,
  replace: (match: RegExpExecArray, text: string) => Replacement | undefined,
): InlineNode[] {
  return nodes.flatMap((node): InlineNode[] => {
    if (node.kind !== "text") return [node];

    const out: InlineNode[] = [];
    let cursor = 0;
    for (const match of node.text.matchAll(pattern)) {
      const found = replace(match, node.text);
      if (found === undefined) continue;
      if (match.index > cursor) out.push({ kind: "text", text: node.text.slice(cursor, match.index) });
      out.push(found.node);
      cursor = match.index + found.length;
    }
    if (out.length === 0) return [node];
    if (cursor < node.text.length) out.push({ kind: "text", text: node.text.slice(cursor) });
    return out;
  });
}

/**
 * The stretches of `content` that are bare URLs, as offsets. The grammar reads
 * them as plain text, so the markers inside an address stay part of it
 * (`https://x/pkg/__init__.py` is one link) while `**https://x/a**` is still a
 * bold link. A `]` ends the stretch, so a URL can sit in a link's label.
 */
function bareUrls(content: string): [start: number, end: number][] {
  const stretches: [number, number][] = [];
  for (const match of content.matchAll(URL_RUN)) {
    const bracket = match[0].indexOf("]");
    const core = trimUrl(bracket < 0 ? match[0] : match[0].slice(0, bracket), TRAILING_IN_FORMATTING);
    if (core.length > 0) stretches.push([match.index, match.index + core.length]);
  }
  return stretches;
}

function linkify(nodes: readonly InlineNode[]): InlineNode[] {
  return replaceInText(nodes, URL_RUN, (match) => {
    const href = trimUrl(match[0]);
    return href.length === 0 ? undefined : { node: { kind: "link", href, text: href }, length: href.length };
  });
}

/** Resolves real members only; any other `@word` remains ordinary text. */
function mentionify(
  nodes: InlineNode[],
  mentions: ReadonlyMap<string, MentionTarget> | undefined,
): InlineNode[] {
  if (mentions === undefined || mentions.size === 0) return nodes;

  return replaceInText(nodes, MENTION_RUN, (match, text) => {
    // "mail@example.com" and "@ada.x" are not a mention of "ada".
    const before = text[match.index - 1];
    const after = text[match.index + match[0].length];
    if (before !== undefined && USERNAME_CHARACTER.test(before)) return undefined;
    if (after !== undefined && USERNAME_CHARACTER.test(after)) return undefined;

    const target = mentions.get((match[1] ?? "").toLocaleLowerCase());
    if (target === undefined) return undefined;
    return {
      node: {
        kind: "mention",
        text: match[0],
        userId: target.userId,
        username: target.username,
        displayName: target.displayName,
        self: target.self === true,
      },
      length: match[0].length,
    };
  });
}

function emojify(
  nodes: InlineNode[],
  emoji: ReadonlyMap<string, string> | undefined,
): InlineNode[] {
  if (emoji === undefined || emoji.size === 0) return nodes;

  return replaceInText(nodes, SHORTCODE, (match) => {
    const shortcode = match[1] ?? "";
    const src = emoji.get(shortcode);
    return src === undefined
      ? undefined
      : { node: { kind: "emoji", shortcode, src }, length: match[0].length };
  });
}

/* -------------------------------------------------------------- formatting */

/** Nesting deeper than this is shown as the text it was typed as. */
const MAX_DEPTH = 32;

export function inlineText(nodes: readonly InlineNode[]): string {
  return nodes
    .map((node) => {
      if (node.kind === "emoji") return `:${node.shortcode}:`;
      return node.kind === "edited" ? "" : node.text;
    })
    .join("");
}

/** The nodes and everything nested inside them, so mentions in emphasis are found too. */
export function inlineDescendants(nodes: readonly InlineNode[]): InlineNode[] {
  return nodes.flatMap((node) => [
    node,
    ...("children" in node && node.children ? inlineDescendants(node.children) : []),
  ]);
}

function nodesOf(spans: readonly Span[], options: ParseInlineOptions): InlineNode[] {
  const nodes: InlineNode[] = [];
  const append = (node: InlineNode) => {
    const previous = nodes.at(-1);
    if (node.kind === "text" && previous?.kind === "text") previous.text += node.text;
    else nodes.push(node);
  };
  // Neighbouring text is joined before the recognisers read it, so a `_` that
  // paired with nothing does not cut `http://a/b_c` or `@jo_x` in two.
  let plain = "";
  const flush = () => {
    if (plain.length === 0) return;
    const linked = linkify([{ kind: "text", text: plain }]);
    emojify(mentionify(linked, options.mentions), options.emoji).forEach(append);
    plain = "";
  };

  for (const span of spans) {
    if (span.type === "text") {
      plain += span.text;
      continue;
    }
    flush();
    switch (span.type) {
      case "verbatim":
        append({ kind: "text", text: span.text });
        break;
      case "code":
        append({ kind: "code", text: span.text });
        break;
      case "link":
        // Labels are plain: no mentions or emoji inside a link's own text.
        append({ kind: "link", href: span.href, text: span.label });
        break;
      case "styled": {
        const children = nodesOf(span.children, options);
        append({ kind: span.style, text: inlineText(children), children });
        break;
      }
    }
  }
  flush();
  return nodes;
}

/**
 * Parses a body into nodes. Only the grammar in `inlineGrammar.ts` is
 * formatting; links must be http(s), and HTML, images and anything else the
 * sender typed stay literal text.
 */
export function parseInline(content: string, options: ParseInlineOptions = {}): InlineNode[] {
  try {
    return nodesOf(readInline(content, MAX_DEPTH, bareUrls(content)), options);
  } catch {
    // Whatever goes wrong, the message must remain readable.
    return [{ kind: "text", text: content }];
  }
}

/* ------------------------------------------------------------ back to text */

/** What would read as markup if a text node were written back as it is. */
const MARKUP_CHARACTER = /[\\`*_~|[\]]/g;
/** Where a mention could start, so text that is not one stays text. */
const MENTION_START = /@(?=[\p{L}\p{N}_.-]{2,32})/gu;
/** Where a custom emoji shortcode could start, for the same reason. */
const SHORTCODE_START = /:(?=[a-zA-Z0-9_-]{2,64}:)/g;

const WRAPPERS = { strong: "**", emphasis: "*", strike: "~~", spoiler: "||" } as const;

function escapeMarkup(text: string): string {
  return text
    .replace(MARKUP_CHARACTER, "\\$&")
    .replace(MENTION_START, "\\@")
    .replace(SHORTCODE_START, "\\:")
    // A colon written after an http scheme makes an address; breaking the
    // scheme keeps text that only looks like one as the text it was.
    .replace(/(https?):\/\//gi, "$1\\://");
}

/** Whether a bare address reads back as exactly the same link. */
function isBareSource(href: string): boolean {
  const [only, ...rest] = bareUrls(href);
  return rest.length === 0 && only !== undefined && only[0] === 0 && only[1] === href.length;
}

/** A code span around `code`: a fence longer than any run of backticks inside it. */
export function codeSource(code: string): string {
  let longest = 0;
  for (const run of code.match(/`+/g) ?? []) longest = Math.max(longest, run.length);
  const fence = "`".repeat(longest + 1);
  // The space a parser strips is added where a backtick, or a space it would
  // strip, touches the fence.
  const spaced = code.startsWith(" ") && code.endsWith(" ") && /[^ ]/.test(code);
  const pad = code.startsWith("`") || code.endsWith("`") || spaced ? " " : "";
  return `${fence}${pad}${code}${pad}${fence}`;
}

type StyledNode = Extract<InlineNode, { kind: "strong" | "emphasis" | "strike" | "spoiler" }>;

function styled(node: InlineNode): node is StyledNode {
  return node.kind === "strong" || node.kind === "emphasis" ||
    node.kind === "strike" || node.kind === "spoiler";
}

/** The inside of a styled node, as markup of its own. */
function styledSource(node: StyledNode): string {
  return node.children === undefined ? escapeMarkup(node.text) : inlineSource(node.children);
}

/**
 * Markup that renders back into `nodes`: what the translator is given, so the
 * formatting survives the round trip. Literal markers are escaped, code gets a
 * fence it cannot contain, and a link keeps its address.
 *
 * Two neighbouring runs of one style are written as one run. Their wrappers
 * would otherwise touch and read as a longer delimiter run, which is not the
 * same markup. Merging styles that render the same way is invisible; the
 * translated message keeps both words bold, struck or hidden.
 */
export function inlineSource(nodes: readonly InlineNode[]): string {
  const source: string[] = [];
  for (let index = 0; index < nodes.length; index += 1) {
    const node = nodes[index] as InlineNode;
    if (styled(node)) {
      let inner = styledSource(node);
      while (index + 1 < nodes.length && nodes[index + 1]?.kind === node.kind) {
        index += 1;
        inner += styledSource(nodes[index] as StyledNode);
      }
      source.push(`${WRAPPERS[node.kind]}${inner}${WRAPPERS[node.kind]}`);
      continue;
    }

    switch (node.kind) {
      case "text":
        source.push(escapeMarkup(node.text));
        break;
      case "code":
        source.push(codeSource(node.text));
        break;
      case "link":
        // A bare address is shorter, but only when it reads back unchanged:
        // one ending in `.` or `*` would lose that character on the way in.
        source.push(
          node.text === node.href && isBareSource(node.href)
            ? node.href
            : `[${escapeMarkup(node.text)}](${node.href})`,
        );
        break;
      case "mention":
        source.push(node.text);
        break;
      case "emoji":
        source.push(`:${node.shortcode}:`);
        break;
      case "edited":
        break;
    }
  }
  return source.join("");
}
