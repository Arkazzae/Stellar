import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";

import { describe, expect, it } from "vitest";

import { inlineSource, parseInline, type InlineNode, type MentionTarget } from "./inline";
import { toggleMarker, type TextEdit } from "./composerText";
import { mentionsUsername } from "./messageBody";

type ExpectedNode =
  | { text: string }
  | { code: string }
  | { strong: ExpectedNode[] }
  | { emphasis: ExpectedNode[] }
  | { strike: ExpectedNode[] }
  | { spoiler: ExpectedNode[] }
  | { link: { href: string; text: string } }
  | { mention: string };

interface MessageCase {
  name: string;
  content: string;
  mentions: string[];
  inline: ExpectedNode[];
  source?: string;
}

interface ComposerCase {
  name: string;
  text: string;
  start: number;
  end: number;
  marker: string;
  expected: TextEdit;
}

interface Contract {
  members: string[];
  messages: MessageCase[];
  composer: ComposerCase[];
}

/** The corpus lives at the repository root, beside the web and server trees. */
const CONTRACT_PATH = [
  join(process.cwd(), "../tests/contracts/message-syntax.json"),
  join(process.cwd(), "tests/contracts/message-syntax.json"),
].find(existsSync);

function fail(message: string): never {
  throw new Error(`message-syntax contract: ${message}`);
}

function stringField(value: Record<string, unknown>, key: string, where: string): string {
  const field = value[key];
  if (typeof field !== "string") fail(`${where} needs a string "${key}"`);
  return field;
}

function numberField(value: Record<string, unknown>, key: string, where: string): number {
  const field = value[key];
  if (typeof field !== "number" || !Number.isInteger(field)) fail(`${where} needs an integer "${key}"`);
  return field;
}

function stringArray(value: unknown, where: string): string[] {
  if (!Array.isArray(value) || value.some((item) => typeof item !== "string")) {
    fail(`${where} needs an array of strings`);
  }
  return value;
}

function checkNode(value: unknown, where: string): ExpectedNode {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    fail(`${where} needs a node object`);
  }
  const node = value as Record<string, unknown>;
  const keys = Object.keys(node);
  if (keys.length !== 1) fail(`${where} needs exactly one node kind`);
  const key = keys[0] ?? "";

  if (key === "text" || key === "code" || key === "mention") {
    return { [key]: stringField(node, key, where) } as ExpectedNode;
  }
  if (key === "strong" || key === "emphasis" || key === "strike" || key === "spoiler") {
    return { [key]: checkNodes(node[key], `${where}.${key}`) } as ExpectedNode;
  }
  if (key === "link") {
    const link = node.link;
    if (link === null || typeof link !== "object" || Array.isArray(link)) {
      fail(`${where}.link needs an object`);
    }
    const fields = link as Record<string, unknown>;
    return {
      link: {
        href: stringField(fields, "href", `${where}.link`),
        text: stringField(fields, "text", `${where}.link`),
      },
    };
  }
  return fail(`${where} has unknown node kind "${key}"`);
}

function checkNodes(value: unknown, where: string): ExpectedNode[] {
  if (!Array.isArray(value)) fail(`${where} needs an array of nodes`);
  return value.map((node, index) => checkNode(node, `${where}[${index}]`));
}

function checkEdit(value: unknown, where: string): TextEdit {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    fail(`${where} needs an object`);
  }
  const edit = value as Record<string, unknown>;
  return {
    text: stringField(edit, "text", where),
    selectionStart: numberField(edit, "selectionStart", where),
    selectionEnd: numberField(edit, "selectionEnd", where),
  };
}

function validate(value: unknown): Contract {
  if (value === null || typeof value !== "object" || Array.isArray(value)) fail("the root needs an object");
  const root = value as Record<string, unknown>;
  const members = stringArray(root.members, "members");
  if (members.length === 0) fail("members must not be empty");
  const memberNames = new Set(members.map((username) => username.toUpperCase()));
  if (memberNames.size !== members.length) fail("members must be unique");
  if (!Array.isArray(root.messages) || root.messages.length === 0) fail("messages must not be empty");
  if (!Array.isArray(root.composer) || root.composer.length === 0) fail("composer must not be empty");

  const names = new Set<string>();
  const uniqueName = (name: string, where: string): string => {
    if (name.length === 0) fail(`${where} needs a name`);
    if (names.has(name)) fail(`duplicate case name "${name}"`);
    names.add(name);
    return name;
  };

  const messages = root.messages.map((value, index): MessageCase => {
    const where = `messages[${index}]`;
    if (value === null || typeof value !== "object" || Array.isArray(value)) fail(`${where} needs an object`);
    const message = value as Record<string, unknown>;
    const source = message.source;
    if (source !== undefined && typeof source !== "string") fail(`${where}.source must be a string`);
    const mentions = stringArray(message.mentions, `${where}.mentions`).map((mention, index) => {
      const normalized = mention.toUpperCase();
      if (mention !== normalized) fail(`${where}.mentions[${index}] must be upper case`);
      if (!memberNames.has(normalized)) fail(`${where}.mentions[${index}] names a member who is not in members`);
      return normalized;
    });
    if (new Set(mentions).size !== mentions.length) fail(`${where}.mentions must be unique`);
    return {
      name: uniqueName(stringField(message, "name", where), where),
      content: stringField(message, "content", where),
      mentions,
      inline: checkNodes(message.inline, `${where}.inline`),
      ...(source === undefined ? {} : { source }),
    };
  });

  const composer = root.composer.map((value, index): ComposerCase => {
    const where = `composer[${index}]`;
    if (value === null || typeof value !== "object" || Array.isArray(value)) fail(`${where} needs an object`);
    const edit = value as Record<string, unknown>;
    return {
      name: uniqueName(stringField(edit, "name", where), where),
      text: stringField(edit, "text", where),
      start: numberField(edit, "start", where),
      end: numberField(edit, "end", where),
      marker: stringField(edit, "marker", where),
      expected: checkEdit(edit.expected, `${where}.expected`),
    };
  });

  return { members, messages, composer };
}

function loadContract(): Contract {
  if (CONTRACT_PATH === undefined) fail("cannot find tests/contracts/message-syntax.json");
  let raw: string;
  try {
    raw = readFileSync(CONTRACT_PATH, "utf8");
  } catch (error) {
    fail(`cannot read ${CONTRACT_PATH}: ${(error as Error).message}`);
  }
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch (error) {
    fail(`not valid JSON: ${(error as Error).message}`);
  }
  return validate(parsed);
}

const contract = loadContract();
const PEOPLE = new Map<string, MentionTarget>(
  contract.members.map((username) => [
    username,
    { userId: `u-${username}`, username, displayName: username, self: false },
  ]),
);

function node(expected: ExpectedNode, where: string): InlineNode {
  if ("text" in expected) return { kind: "text", text: expected.text };
  if ("code" in expected) return { kind: "code", text: expected.code };
  if ("mention" in expected) {
    const username = expected.mention.slice(1).toLocaleLowerCase();
    const target = PEOPLE.get(username);
    if (target === undefined) fail(`${where} mentions @${username}, which is not in members`);
    return { kind: "mention", text: expected.mention, ...target, self: false };
  }
  if ("link" in expected) {
    return { kind: "link", href: expected.link.href, text: expected.link.text };
  }
  const styled = (
    kind: "strong" | "emphasis" | "strike" | "spoiler",
    children: ExpectedNode[],
  ): InlineNode => {
    const resolved = children.map((child, index) => node(child, `${where}.${kind}[${index}]`));
    return { kind, text: expectedText(children), children: resolved };
  };
  if ("strong" in expected) return styled("strong", expected.strong);
  if ("emphasis" in expected) return styled("emphasis", expected.emphasis);
  if ("strike" in expected) return styled("strike", expected.strike);
  if ("spoiler" in expected) return styled("spoiler", expected.spoiler);
  return fail(`${where} has an unknown kind`);
}

function nodes(expected: ExpectedNode[], where: string): InlineNode[] {
  return expected.map((value, index) => node(value, `${where}[${index}]`));
}

/** The visible text the corpus itself spells out, without the production helper. */
function expectedText(expected: ExpectedNode[]): string {
  return expected
    .map((value) => {
      if ("text" in value) return value.text;
      if ("code" in value) return value.code;
      if ("mention" in value) return value.mention;
      if ("link" in value) return value.link.text;
      if ("strong" in value) return expectedText(value.strong);
      if ("emphasis" in value) return expectedText(value.emphasis);
      if ("strike" in value) return expectedText(value.strike);
      return expectedText(value.spoiler);
    })
    .join("");
}

describe("shared message-syntax contract", () => {
  describe("messages", () => {
    for (const message of contract.messages) {
      it(message.name, () => {
        const parsed = parseInline(message.content, { mentions: PEOPLE });
        expect(parsed).toEqual(nodes(message.inline, message.name));

        if (message.source !== undefined) {
          expect(inlineSource(parsed)).toBe(message.source);
        }

        // The API checks the same field. Ask the transcript's own mention
        // predicate about every member, so a case cannot claim a mention that
        // only one side's parser would find.
        const detected = contract.members
          .filter((username) => mentionsUsername(message.content, username))
          .map((username) => username.toUpperCase())
          .sort();
        expect(detected, message.name).toEqual([...message.mentions].sort());
      });
    }
  });

  describe("composer", () => {
    for (const edit of contract.composer) {
      it(edit.name, () => {
        expect(toggleMarker(edit.text, edit.start, edit.end, edit.marker)).toEqual(edit.expected);
      });
    }
  });
});
