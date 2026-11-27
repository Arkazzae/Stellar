import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";

import type { CustomEmoji } from "@/ports";

import { pushRecentEmoji, readRecentEmoji } from "./emoji";
import {
  forgetQuickReactionOrder,
  keepPlaces,
  quickReactionsFor,
  rememberReaction,
  useQuickReactions,
} from "./quickReactions";

const MUSHRIS: CustomEmoji = { id: "e-1", shortcode: ":mushris:", glyph: "https://cdn.example/m.png" };

const reactionsOf = (row: readonly { reaction: string }[]) => row.map(({ reaction }) => reaction);

beforeEach(() => {
  localStorage.clear();
  forgetQuickReactionOrder();
});

describe("the quick reaction row", () => {
  it("offers thumbs up, heart and tears of joy before anything was picked", () => {
    expect(reactionsOf(quickReactionsFor([], []))).toEqual(["👍", "❤️", "😂"]);
  });

  it("puts the recents first, newest first, and fills up without repeating one", () => {
    expect(reactionsOf(quickReactionsFor(["zaba"], []))).toEqual(["🐸", "👍", "❤️"]);
    expect(reactionsOf(quickReactionsFor(["serce", "zaba"], []))).toEqual(["❤️", "🐸", "👍"]);
    expect(reactionsOf(quickReactionsFor(["ogien", "zaba", "oczko", "serce"], []))).toEqual([
      "🔥",
      "🐸",
      "😉",
    ]);
  });

  it("offers a server emoji as its shortcode, with its picture, while the server has it", () => {
    expect(quickReactionsFor(["mushris"], [MUSHRIS])[0]).toEqual({
      reaction: ":mushris:",
      imageUrl: "https://cdn.example/m.png",
    });
    // Deleted from the server since: the next recent takes its place.
    expect(reactionsOf(quickReactionsFor(["mushris", "zaba"], []))).toEqual(["🐸", "👍", "❤️"]);
  });

  it("skips a recent nothing answers to any more", () => {
    expect(reactionsOf(quickReactionsFor(["no_such_emoji"], []))).toEqual(["👍", "❤️", "😂"]);
  });
});

describe("keeping the row's places", () => {
  const row = (...reactions: string[]) => reactions.map((reaction) => ({ reaction }));

  it("keeps the old order while the row holds the same emoji", () => {
    expect(reactionsOf(keepPlaces(row("😂", "👍", "❤️"), row("👍", "❤️", "😂")))).toEqual([
      "👍",
      "❤️",
      "😂",
    ]);
  });

  it("starts newest first again once an emoji joins the row", () => {
    expect(reactionsOf(keepPlaces(row("🐸", "👍", "❤️"), row("👍", "❤️", "😂")))).toEqual([
      "🐸",
      "👍",
      "❤️",
    ]);
  });

  it("follows a press from the row without moving it, and every reader agrees", () => {
    const first = renderHook(() => useQuickReactions([]));
    expect(reactionsOf(first.result.current)).toEqual(["👍", "❤️", "😂"]);

    // The third one pressed: it is now the newest recent.
    act(() => rememberReaction("😂"));
    expect(readRecentEmoji()[0]).toBe("lzy_smiechu");
    expect(reactionsOf(first.result.current)).toEqual(["👍", "❤️", "😂"]);

    // Another toolbar drawn now offers the same places.
    const second = renderHook(() => useQuickReactions([]));
    expect(reactionsOf(second.result.current)).toEqual(["👍", "❤️", "😂"]);

    // A pick from the picker brings in a new one, newest first.
    act(() => void pushRecentEmoji("zaba"));
    expect(reactionsOf(first.result.current)).toEqual(["🐸", "😂", "👍"]);
    expect(reactionsOf(second.result.current)).toEqual(["🐸", "😂", "👍"]);
  });
});

describe("remembering a reaction", () => {
  it("keeps a built-in one by its shortcode, with or without the presentation selector", () => {
    rememberReaction("🐸");
    rememberReaction("❤");
    expect(readRecentEmoji()).toEqual(["serce", "zaba"]);
  });

  it("keeps a server emoji by its bare shortcode", () => {
    rememberReaction(":mushris:");
    expect(readRecentEmoji()).toEqual(["mushris"]);
  });

  it("leaves out an emoji the picker does not have", () => {
    rememberReaction("🫨🫨");
    expect(readRecentEmoji()).toEqual([]);
  });
});
