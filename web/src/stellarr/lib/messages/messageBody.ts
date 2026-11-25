/**
 * The parts a message's own text turns into: prose, and fenced code blocks.
 *
 * Fences arrive inside `content` because `Attachment` has no code kind: a paste
 * is text, not a file.
 */

import { resolveCodeLanguage, tokenizeCode } from "./codeLanguages";
import {
  inlineDescendants,
  inlineText,
  parseInline,
  type InlineNode,
  type MentionTarget,
  type ParseInlineOptions,
} from "./inline";
import type { MessagePart } from "./partTypes";

/** What ends a line, as `^` and `$` read it in a multiline pattern. */
function isLineEnd(char: string | undefined): boolean {
  return char === "\n" || char === "\r" || char === "\u2028" || char === "\u2029";
}

/** Where the line starting at `from` ends: its line break, or the end of the body. */
function lineEnd(content: string, from: number): number {
  let end = from;
  while (end < content.length && !isLineEnd(content[end])) end += 1;
  return end;
}

interface Fence {
  start: number;
  end: number;
  /** The info string: a lexer alias or, as in the mockup, a file name. */
  info: string;
  source: string;
}

/** A longer opening run can still be a legacy triple fence with extra info. */
function lastFenceClosings(content: string): Map<number, number> {
  const closings = new Map<number, number>();
  for (const match of content.matchAll(/^`{4,}(?=[ \t]*$)/gm)) {
    closings.set(match[0].length, match.index);
  }
  return closings;
}

/**
 * The fenced blocks of a body, in order. A fence is at least three backticks
 * and an info string at the start of a line. Its closing line uses the same
 * number of backticks followed by optional spaces or tabs. Older messages
 * with extra opening backticks and a triple closing fence still render.
 *
 * Read line by line rather than with a lazy pattern, which would look for a
 * closing line once per opening one. Here a fence that finds no closing line
 * ends the search: no later fence could find one either.
 */
export function fences(content: string): Fence[] {
  const found: Fence[] = [];
  let closings: Map<number, number> | undefined;
  let line = 0;
  while (line < content.length) {
    const infoEnd = lineEnd(content, line);
    if (!content.startsWith("```", line) || content[infoEnd] !== "\n") {
      line = infoEnd + 1;
      continue;
    }

    let markerEnd = line + 3;
    while (content[markerEnd] === "`") markerEnd += 1;
    if (markerEnd > line + 3) {
      closings ??= lastFenceClosings(content);
      if ((closings.get(markerEnd - line) ?? -1) <= line) markerEnd = line + 3;
    }
    const marker = content.slice(line, markerEnd);
    const sourceStart = infoEnd + 1;
    let closing = sourceStart;
    let closingEnd = lineEnd(content, closing);
    while (!(content.startsWith(marker, closing) && /^[ \t]*$/.test(content.slice(closing + marker.length, closingEnd)))) {
      if (closingEnd >= content.length) return found;
      closing = closingEnd + 1;
      closingEnd = lineEnd(content, closing);
    }

    // The line break before the closing ``` belongs to neither side.
    const sourceEnd = closing > sourceStart && content[closing - 1] === "\n" ? closing - 1 : closing;
    found.push({
      start: line,
      end: closingEnd,
      info: content.slice(markerEnd, infoEnd),
      source: content.slice(sourceStart, sourceEnd),
    });
    line = closingEnd + 1;
  }
  return found;
}

type BodySegment = { kind: "prose"; text: string } | { kind: "code"; fence: Fence };

/** A body cut into its prose and its fenced blocks, in order; prose is trimmed and never empty. */
function segments(content: string): BodySegment[] {
  const out: BodySegment[] = [];
  let cursor = 0;
  const prose = (raw: string): void => {
    const text = raw.trim();
    if (text.length > 0) out.push({ kind: "prose", text });
  };
  for (const fence of fences(content)) {
    prose(content.slice(cursor, fence.start));
    out.push({ kind: "code", fence });
    cursor = fence.end;
  }
  prose(content.slice(cursor));
  return out;
}

/** Splits a body into its prose and its fenced blocks, in order. */
function textAndCodeParts(content: string, options: ParseInlineOptions): MessagePart[] {
  return segments(content).map((segment): MessagePart => {
    if (segment.kind === "prose") return { kind: "text", nodes: parseInline(segment.text, options) };
    const authored = segment.fence.info.trim();
    const { source } = segment.fence;
    const language = resolveCodeLanguage(authored, source);
    return {
      kind: "code",
      lang: language === "plaintext" ? "" : authored,
      source,
      language,
      tokens: tokenizeCode(source, language),
    };
  });
}

/**
 * Whether the body mentions `username` as the transcript would show it. The
 * same parse decides the highlight and the alert, so a name in code, in a
 * link's label or in a URL calls nobody.
 */
export function mentionsUsername(content: string, username: string): boolean {
  const target: MentionTarget = { userId: "", username, displayName: username };
  const mentions = new Map([[username.toLocaleLowerCase(), target]]);
  return segments(content).some(
    (segment) =>
      segment.kind === "prose" &&
      inlineDescendants(parseInline(segment.text, { mentions })).some((node) => node.kind === "mention"),
  );
}

/**
 * Joins the text on either side of a link that was just taken out. Spaces and
 * tabs around the gap collapse into one; line breaks are left alone.
 */
function closeGap(kept: InlineNode[], text: string): void {
  const previous = kept.at(-1);
  if (previous?.kind !== "text") {
    const tail = text.replace(/^[ \t]+/, "");
    if (tail.length > 0) kept.push({ kind: "text", text: tail });
    return;
  }

  const spaced = /[ \t]$/.test(previous.text) || /^[ \t]/.test(text);
  previous.text = previous.text.replace(/[ \t]+$/, "");
  const tail = text.replace(/^[ \t]+/, "");
  previous.text += spaced && previous.text.length > 0 && tail.length > 0 ? ` ${tail}` : tail;
}

/** The nodes without the links in `urls`, and without what that leaves empty. */
function withoutLinks(nodes: readonly InlineNode[], urls: ReadonlySet<string>): InlineNode[] {
  const kept: InlineNode[] = [];
  let afterGap = false;

  for (const original of nodes) {
    let node = original;
    if ("children" in node && node.children) {
      const children = withoutLinks(node.children, urls);
      if (children.length === 0) {
        afterGap = true;
        continue;
      }
      node = { ...node, children, text: inlineText(children) };
    }
    if (node.kind === "link" && urls.has(node.href)) {
      afterGap = true;
      continue;
    }

    if (node.kind === "text" && afterGap) closeGap(kept, node.text);
    else kept.push({ ...node });
    afterGap = false;
  }

  const last = kept.at(-1);
  if (afterGap && last?.kind === "text") last.text = last.text.trimEnd();
  return kept.filter((node) => node.kind !== "text" || node.text.length > 0);
}

/**
 * A URL is hidden only when a real card or player for that exact URL is
 * rendered next to the text. The authored content is untouched; this is
 * presentation, the same as swapping the URL for its preview.
 */
function withoutPreviewedLinks(parts: MessagePart[], previewed: ReadonlySet<string>): MessagePart[] {
  if (previewed.size === 0) return parts;
  return parts.flatMap((part): MessagePart[] => {
    if (part.kind !== "text") return [part];
    const nodes = withoutLinks(part.nodes, previewed);
    return nodes.length === 0 ? [] : [{ kind: "text", nodes }];
  });
}

/** The prose and code blocks of `content`, minus links that have a preview of their own. */
export function bodyParts(
  content: string,
  options: ParseInlineOptions,
  previewedUrls: ReadonlySet<string>,
): MessagePart[] {
  return withoutPreviewedLinks(textAndCodeParts(content, options), previewedUrls);
}
