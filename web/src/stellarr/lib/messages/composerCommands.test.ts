import { describe, expect, it } from "vitest";

import { matchCommand, slashQuery } from "./composerCommands";

const marked = (query: string, name: string, alias: string) => {
  const match = matchCommand(query, { name, alias });
  if (match === null) return null;
  const text = match.via === "name" ? name : alias;
  return { via: match.via, marked: text.slice(match.from, match.to), rank: match.rank };
};

describe("matching a command", () => {
  it("ignores diacritics both ways, marking the letters as the name writes them", () => {
    expect(marked("glos", "głosówka", "voice")).toMatchObject({ via: "name", marked: "głos" });
    expect(marked("GŁOSÓ", "głosówka", "voice")).toMatchObject({ marked: "głosó" });
    expect(marked("przypiete", "przypięte", "pinned")).toMatchObject({ marked: "przypięte" });
  });

  it("answers to the English name in every language", () => {
    expect(marked("pin", "przypięte", "pinned")).toMatchObject({ via: "alias", marked: "pin" });
    expect(marked("kick", "entfernen", "kick")).toMatchObject({ via: "alias", marked: "kick" });
  });

  it("ranks a name that starts with the query over one that holds it, and that over letters in order", () => {
    const prefix = marked("za", "zaplanuj", "schedule");
    const inside = marked("banuj", "zbanuj", "ban");
    // As drawn: "za" finds "zbanuj", and only the "z" that agrees from the start is marked.
    const inOrder = marked("za", "zbanuj", "ban");
    expect(inOrder).toMatchObject({ marked: "z" });
    expect(inside).toMatchObject({ marked: "banuj" });
    expect(prefix!.rank).toBeLessThan(inside!.rank);
    expect(inside!.rank).toBeLessThan(inOrder!.rank);
    expect(marked("ba", "zbanuj", "ban")).toMatchObject({ via: "alias" });
    expect(marked("xyz", "zbanuj", "ban")).toBeNull();
  });
});

describe("the command word under the caret", () => {
  it("is only the word after a slash that opens the draft", () => {
    expect(slashQuery("/gif", 4)).toEqual({ query: "gif", end: 4 });
    expect(slashQuery("/gif kot", 2)).toEqual({ query: "gif", end: 4 });
    // A slash in the middle of a sentence, or the caret past the word, is writing.
    expect(slashQuery("hej /gif", 8)).toBeNull();
    expect(slashQuery("/gif kot", 8)).toBeNull();
    expect(slashQuery("/gif", 0)).toBeNull();
  });
});
