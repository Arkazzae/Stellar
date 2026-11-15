import { describe, expect, it } from "vitest";

import { FORMAT_MARKERS, fencedCode, insertEmojiAt, toggleMarker } from "./composerText";
import { inlineText, parseInline, type InlineNode } from "./inline";

/** The node kind each panel button should produce. */
const STYLES = {
  "**": "strong",
  "*": "emphasis",
  "~~": "strike",
  "||": "spoiler",
  "`": "code",
} as const satisfies Record<(typeof FORMAT_MARKERS)[number], InlineNode["kind"]>;

/** The first formatted node of the result, which is what the button is for. */
function formatted(source: string): InlineNode {
  const node = parseInline(source).find((candidate) => candidate.kind !== "text");
  if (node === undefined) throw new Error(`no formatting in ${JSON.stringify(source)}`);
  return node;
}

describe("toggleMarker", () => {
  it.each(FORMAT_MARKERS)("wraps a plain selection with %j", (marker) => {
    const edit = toggleMarker("middle", 0, 6, marker);

    expect(edit).toEqual({
      text: `${marker}middle${marker}`,
      selectionStart: marker.length,
      selectionEnd: marker.length + 6,
    });
    expect(formatted(edit.text)).toMatchObject({
      kind: STYLES[marker],
      text: "middle",
    });
  });

  it.each(FORMAT_MARKERS)("keeps whitespace outside the %j markers", (marker) => {
    const source = "  foo  ";
    const edit = toggleMarker(source, 0, source.length, marker);

    if (marker === "`") {
      // Inline code keeps its spaces: padding inside the fence makes the
      // parser strip its own space, not one the sender selected.
      expect(edit.text).toBe("`   foo   `");
      expect(formatted(edit.text)).toMatchObject({ text: "  foo  " });
    } else {
      expect(edit.text).toBe(`  ${marker}foo${marker}  `);
      expect(formatted(edit.text)).toMatchObject({ text: "foo" });
    }
    expect(edit.selectionStart).toBeGreaterThanOrEqual(0);
    expect(edit.selectionEnd).toBeLessThanOrEqual(edit.text.length);
    expect(toggleMarker(edit.text, edit.selectionStart, edit.selectionEnd, marker).text)
      .toBe(source);
  });

  it.each(FORMAT_MARKERS)("keeps punctuation inside the %j markers", (marker) => {
    const edit = toggleMarker("Ocena!", 0, 6, marker);

    expect(edit.text).toBe(`${marker}Ocena!${marker}`);
    expect(formatted(edit.text)).toMatchObject({ text: "Ocena!" });
  });

  it.each(FORMAT_MARKERS)("formats a selection inside a word with %j", (marker) => {
    const edit = toggleMarker("wording", 1, 4, marker);

    expect(edit.text).toBe(`w${marker}ord${marker}ing`);
    expect(formatted(edit.text)).toMatchObject({ text: "ord" });
  });

  it("widens punctuation that a neighbouring word would block", () => {
    // `**` after a word may not be followed by punctuation, so the run grows
    // to the word characters beside the selection rather than showing stars.
    expect(toggleMarker("foo.bar", 3, 6, "**").text).toBe("fo**o.ba**r");
    expect(toggleMarker("foo.bar", 2, 4, "**").text).toBe("fo**o.b**ar");
    expect(formatted("fo**o.ba**r")).toMatchObject({ kind: "strong", text: "o.ba" });
    expect(formatted("fo**o.b**ar")).toMatchObject({ kind: "strong", text: "o.b" });

    // Spoilers and code have no flanking rule, so they take the punctuation as is.
    expect(toggleMarker("foo.bar", 3, 6, "||").text).toBe("foo||.ba||r");
    expect(toggleMarker("foo.bar", 3, 6, "`").text).toBe("foo`.ba`r");
    expect(formatted("foo||.ba||r")).toMatchObject({ kind: "spoiler", text: ".ba" });
    expect(formatted("foo`.ba`r")).toMatchObject({ kind: "code", text: ".ba" });
  });

  it("widens a punctuation-only selection instead of leaving unusable stars", () => {
    const edit = toggleMarker("foo.bar", 3, 4, "**");

    expect(edit.text).toBe("fo**o.b**ar");
    expect(edit.text).not.toContain("****");
    expect(formatted(edit.text)).toMatchObject({ kind: "strong", text: "o.b" });
    expect(toggleMarker(edit.text, edit.selectionStart, edit.selectionEnd, "**").text)
      .toBe("foo.bar");
  });

  it("escapes literal marker characters inside a new run", () => {
    const cases = [
      ["a**b", "**", "a**b"],
      ["a*b", "*", "a*b"],
      ["a~~b", "~~", "a~~b"],
      ["a||b", "||", "a||b"],
      // A backslash the sender already typed keeps its meaning: this renders a star.
      ["a\\*b", "**", "a*b"],
    ] as const;
    for (const [content, marker, rendered] of cases) {
      const edit = toggleMarker(content, 0, content.length, marker);
      expect(formatted(edit.text)).toMatchObject({ text: rendered });
      if (!content.includes("\\")) {
        expect(edit.text).not.toBe(`${marker}${content}${marker}`);
      }
      const removed = toggleMarker(edit.text, edit.selectionStart, edit.selectionEnd, marker);
      expect(inlineText(parseInline(removed.text))).toBe(rendered);
      if (!content.includes("\\")) expect(removed.text).toBe(content);
    }
  });

  it("does not let a trailing backslash escape the closing marker", () => {
    for (const [content, marker] of [
      ["a\\", "**"],
      ["a\\", "*"],
      ["a\\", "~~"],
      ["a\\", "||"],
    ] as const) {
      const edit = toggleMarker(content, 0, content.length, marker);
      expect(formatted(edit.text)).toMatchObject({ text: content });
      const removed = toggleMarker(edit.text, edit.selectionStart, edit.selectionEnd, marker);
      expect(inlineText(parseInline(removed.text))).toBe(content);
    }
  });

  it("formats a selection that is only the marker character itself", () => {
    for (const [source, at, marker] of [
      ["foo.*bar", 3, "**"],
      ["foo~bar", 3, "~~"],
      ["foo|bar", 3, "||"],
      ["foo`bar", 3, "`"],
    ] as const) {
      const edit = toggleMarker(source, at, at + 1, marker);
      expect(inlineText(parseInline(edit.text))).toBe(source);
      expect(formatted(edit.text)).toBeDefined();
      const removed = toggleMarker(edit.text, edit.selectionStart, edit.selectionEnd, marker);
      expect(inlineText(parseInline(removed.text))).toBe(source);
    }
  });

  it("gives inline code a fence longer than any run inside the selection", () => {
    const cases = ["a`b", "`foo`", "``", "a``b"];
    for (const source of cases) {
      const edit = toggleMarker(source, 0, source.length, "`");
      expect(formatted(edit.text)).toEqual({ kind: "code", text: source });
      const removed = toggleMarker(edit.text, edit.selectionStart, edit.selectionEnd, "`");
      expect(removed.text).toBe(source);
    }

    expect(toggleMarker("a`b", 0, 3, "`").text).toBe("``a`b``");
    expect(toggleMarker("`foo`", 0, 5, "`").text).toBe("`` `foo` ``");
    expect(toggleMarker("``", 0, 2, "`").text).toBe("``` `` ```");
  });

  it("undoes a wrap when the formatted words are selected again", () => {
    const cases: [text: string, start: number, end: number][] = [
      ["middle", 0, 6],
      ["  foo  ", 0, 7],
      ["a foo b", 1, 6],
      ["Ocena!", 0, 6],
      ["wording", 1, 4],
      ["foo.bar", 3, 6],
      ["foo.bar", 2, 4],
    ];
    for (const marker of FORMAT_MARKERS) {
      for (const [source, start, end] of cases) {
        const once = toggleMarker(source, start, end, marker);
        const twice = toggleMarker(once.text, once.selectionStart, once.selectionEnd, marker);
        expect(twice.text).toBe(source);
      }
    }
  });

  it("inserts an empty pair for a caret and removes it again", () => {
    const inside = toggleMarker("ab", 1, 1, "**");
    expect(inside.text).toBe("a****b");
    expect(inside.selectionStart).toBe(3);
    expect(inside.selectionEnd).toBe(3);

    const removed = toggleMarker(inside.text, inside.selectionStart, inside.selectionEnd, "**");
    expect(removed.text).toBe("ab");
    expect(removed.selectionStart).toBe(1);
  });

  it("leaves a whitespace-only selection as typed around an empty pair", () => {
    const edit = toggleMarker("a   b", 1, 4, "~~");

    expect(edit.text).toBe("a~~~~   b");
    expect(edit.selectionStart).toBe(3);
    expect(edit.selectionEnd).toBe(3);
    expect(toggleMarker(edit.text, edit.selectionStart, edit.selectionEnd, "~~").text).toBe("a   b");
  });

  it("clamps and orders the selection instead of slicing outside the draft", () => {
    expect(toggleMarker("abc", 9, -2, "**")).toEqual({
      text: "**abc**",
      selectionStart: 2,
      selectionEnd: 5,
    });
  });

  it("does nothing without a marker", () => {
    expect(toggleMarker("abc", 1, 2, "")).toEqual({
      text: "abc",
      selectionStart: 1,
      selectionEnd: 2,
    });
  });
});

describe("insertEmojiAt", () => {
  it("spaces the glyph from the words around it", () => {
    expect(insertEmojiAt("hej", 3, "🎉")).toEqual({
      text: "hej 🎉 ",
      selectionStart: 7,
      selectionEnd: 7,
    });
    expect(insertEmojiAt("hej ", 4, "🎉").text).toBe("hej 🎉 ");
  });
});

describe("fencedCode", () => {
  it("puts the language on the opening fence and the source between the fences", () => {
    expect(fencedCode("ts", "const a = 1;")).toBe("```ts\nconst a = 1;\n```");
  });
});
