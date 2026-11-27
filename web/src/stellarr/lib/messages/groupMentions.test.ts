import { describe, expect, it } from "vitest";

import type { Message } from "@/ports";

import { groupMentionCandidates, insertMention, matchMentions, type MentionCandidate } from "./composerMentions";
import { groupOfWord, groupWordFor, isGroupWord, stampedGroups, type GroupMention } from "./groupMentions";
import { inlineDescendants, inlineSource, parseInline, type InlineNode, type MentionTarget } from "./inline";
import { mentionsUsername } from "./messageBody";
import { toParts } from "./parts";

const everyone = new Set<GroupMention>(["everyone"]);

/**
 * The cases the server is held to as well
 * (`api/crates/server/src/messages/mentions/tests.rs`), so that a pill is drawn
 * exactly where the server called a group: content, and the groups it calls on.
 */
const SENTENCE_ENDS: readonly (readonly [string, readonly GroupMention[]])[] = [
  ["Spotkanie o 20 @wszyscy.", ["everyone"]],
  ["@EVERYONE...", ["everyone"]],
  ["Kto jest, @obecni.", ["here"]],
  ["(@here.)", ["here"]],
  ["@everyone. @here.", ["everyone", "here"]],
  ["@wszyscy!", ["everyone"]],
  ["@wszyscy.x", []],
  ["@wszyscy.-", []],
  ["`@wszyscy.`", []],
  ["\\@wszyscy.", []],
];
const both = new Set<GroupMention>(["everyone", "here"]);

function pills(content: string, groups?: ReadonlySet<GroupMention>, mentions?: ReadonlyMap<string, MentionTarget>) {
  return inlineDescendants(parseInline(content, { ...(groups ? { groups } : {}), ...(mentions ? { mentions } : {}) }))
    .filter((node): node is Extract<InlineNode, { kind: "group-mention" | "mention" }> =>
      node.kind === "group-mention" || node.kind === "mention");
}

function message(content: string, stamp: Partial<Pick<Message, "mentionsEveryone" | "mentionsHere">> = {}): Message {
  return {
    id: "m-1",
    channelId: "c-1",
    author: { id: "u-maja", displayName: "Maja" },
    content,
    attachments: [],
    embeds: [],
    createdAt: "2026-11-27T10:00:00Z",
    ...stamp,
  };
}

describe("the words of groups", () => {
  it("are read in either language and in any case", () => {
    expect(["everyone", "Wszyscy", "EVERYONE"].map(groupOfWord)).toEqual(["everyone", "everyone", "everyone"]);
    expect(["here", "OBECNI", "Here"].map(groupOfWord)).toEqual(["here", "here", "here"]);
    expect(["everyones", "obecnie", "ada"].map(groupOfWord)).toEqual([undefined, undefined, undefined]);
    expect(isGroupWord("Wszyscy")).toBe(true);
  });

  it("are written in the reader's language, the English ones for every language but Polish", () => {
    expect(groupWordFor("everyone", "pl")).toBe("wszyscy");
    expect(groupWordFor("here", "pl")).toBe("obecni");
    expect(groupWordFor("everyone", "en")).toBe("everyone");
    expect(groupWordFor("here", "de")).toBe("here");
  });

  it("come from the server's stamp, not from the text", () => {
    expect([...stampedGroups(message("@everyone"))]).toEqual([]);
    expect([...stampedGroups(message("@everyone", { mentionsEveryone: true, mentionsHere: true }))]).toEqual([
      "everyone",
      "here",
    ]);
  });
});

describe("a group's pill", () => {
  it("is drawn only for the groups the message was stamped with", () => {
    expect(pills("Hej @everyone i @here")).toEqual([]);
    expect(pills("Hej @everyone i @here", everyone)).toEqual([
      { kind: "group-mention", text: "@everyone", group: "everyone" },
    ]);
    expect(pills("@Wszyscy, @OBECNI!", both)).toEqual([
      { kind: "group-mention", text: "@Wszyscy", group: "everyone" },
      { kind: "group-mention", text: "@OBECNI", group: "here" },
    ]);
  });

  it("is not drawn in code, after an escape, in a link's label or inside a longer word", () => {
    for (const content of ["`@everyone`", "\\@everyone", "[@everyone](https://example.test)", "a@everyone", "@everyones"]) {
      expect(pills(content, both), content).toEqual([]);
    }
  });

  it("calls the group at the end of a sentence where the server does, and nowhere else", () => {
    for (const [content, called] of SENTENCE_ENDS) {
      const drawn = new Set(pills(content, both).map((node) => (node.kind === "group-mention" ? node.group : "member")));
      expect([...drawn].sort(), content).toEqual([...called].sort());
    }
  });

  it("leaves the dots after the word as text, and draws nothing without the stamp", () => {
    const nodes = parseInline("Spotkanie o 20 @wszyscy.", { groups: everyone });
    expect(nodes).toEqual([
      { kind: "text", text: "Spotkanie o 20 " },
      { kind: "group-mention", text: "@wszyscy", group: "everyone" },
      { kind: "text", text: "." },
    ]);
    expect(inlineSource(nodes)).toBe("Spotkanie o 20 @wszyscy.");
    expect(pills("Spotkanie o 20 @wszyscy.")).toEqual([]);
    expect(pills("@EVERYONE...", everyone)).toEqual([{ kind: "group-mention", text: "@EVERYONE", group: "everyone" }]);
  });

  it("keeps a member whose username is a group's word and a dot, as the server still names them", () => {
    const dotted = new Map([["wszyscy.", { userId: "u-dot", username: "wszyscy.", displayName: "Dot" }]]);

    expect(pills("Spotkanie o 20 @wszyscy.", everyone, dotted)).toEqual([
      { kind: "mention", text: "@wszyscy.", userId: "u-dot", username: "wszyscy.", displayName: "Dot", self: false },
    ]);
    expect(mentionsUsername("Spotkanie o 20 @wszyscy.", "wszyscy.")).toBe(true);
    expect(isGroupWord("wszyscy.")).toBe(false);
  });

  it("wins over a member who took the word as a username before it was reserved", () => {
    const legacy = new Map([["everyone", { userId: "u-old", username: "everyone", displayName: "Old" }]]);

    expect(pills("@everyone", undefined, legacy)).toEqual([]);
    expect(pills("@everyone", everyone, legacy)).toEqual([
      { kind: "group-mention", text: "@everyone", group: "everyone" },
    ]);
    expect(mentionsUsername("@everyone", "everyone")).toBe(false);
  });

  it("is written back as it was typed", () => {
    expect(inlineSource(parseInline("hej @Wszyscy", { groups: everyone }))).toBe("hej @Wszyscy");
  });

  it("is in the parts of a stamped message and not of an unstamped one", () => {
    const nodes = (content: string, stamp = {}) =>
      toParts(message(content, stamp)).flatMap((part) => (part.kind === "text" ? inlineDescendants(part.nodes) : []));

    expect(nodes("@obecni zebranie").some((node) => node.kind === "group-mention")).toBe(false);
    expect(nodes("@obecni zebranie", { mentionsHere: true })).toContainEqual({
      kind: "group-mention",
      text: "@obecni",
      group: "here",
    });
    // The stamp names the group: a message calling on those here does not
    // light up `@everyone` written beside it.
    expect(nodes("@everyone @here", { mentionsHere: true }).filter((node) => node.kind === "group-mention")).toEqual([
      { kind: "group-mention", text: "@here", group: "here" },
    ]);
  });
});

describe("the composer's group entries", () => {
  const people: MentionCandidate[] = [
    { id: "u-ewa", username: "ewa", displayName: "Ewa Nowak" },
    { id: "u-old", username: "everyone", displayName: "Old account" },
    { id: "u-ola", username: "ola", displayName: "Ola Nowak" },
  ];
  const labels = { everyone: "Notify everyone in this channel", here: "Notify everyone online in this channel" };
  const groups = groupMentionCandidates("en", (group) => labels[group]);

  it("come first while the query starts any of a group's words", () => {
    const named = people.filter((candidate) => candidate.username !== "everyone");
    const names = (query: string, locale: "en" | "pl" = "en") =>
      matchMentions(named, query, locale, groups).map((candidate) => candidate.username);

    expect(names("e")).toEqual(["everyone", "ewa"]);
    expect(names("")).toEqual(["everyone", "here", "ewa", "ola"]);
    // The Polish word finds the group in an English interface too.
    expect(names("wsz")).toEqual(["everyone"]);
    // Ewa Nowak is found by her name.
    expect(names("o")).toEqual(["here", "ola", "ewa"]);
    expect(names("x")).toEqual([]);
  });

  it("are not there for whoever may not call on the channel", () => {
    expect(matchMentions(people, "ew", "en").map((candidate) => candidate.username)).toEqual(["ewa"]);
  });

  it("write the group's word in the reader's language", () => {
    const polish = groupMentionCandidates("pl", (group) => (group === "everyone" ? "Powiadom wszystkich na kanale" : "Powiadom osoby online na kanale"));
    expect(polish).toEqual([
      { id: "group:everyone", username: "wszyscy", displayName: "Powiadom wszystkich na kanale", group: "everyone" },
      { id: "group:here", username: "obecni", displayName: "Powiadom osoby online na kanale", group: "here" },
    ]);
    const edit = insertMention("hej @ws", { start: 4, end: 7, query: "ws", key: "4:ws" }, polish[0]!.username);
    expect(edit.text).toBe("hej @wszyscy ");
  });
});
