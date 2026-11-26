import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { KeyChord } from "./keybinds";
import { TalkKey, listenForTalkKey, type TalkChange } from "./talkKey";

const none = { ctrl: false, alt: false, shift: false, meta: false };
const V: KeyChord = { code: "KeyV", ...none };
const CMD_K: KeyChord = { code: "KeyK", ...none, meta: true };

let changes: TalkChange[];
let key: TalkKey;
let stop: () => void;

function dispatch(
  type: "keydown" | "keyup",
  init: KeyboardEventInit,
  target: EventTarget = document.body,
): KeyboardEvent {
  const event = new KeyboardEvent(type, { bubbles: true, cancelable: true, ...init });
  target.dispatchEvent(event);
  return event;
}

function field(tag: "input" | "textarea" | "div" = "input"): HTMLElement {
  const element = document.createElement(tag);
  if (tag === "div") element.setAttribute("contenteditable", "true");
  document.body.append(element);
  return element;
}

beforeEach(() => {
  changes = [];
  key = new TalkKey((change) => changes.push(change));
  stop = listenForTalkKey(key);
  key.configure(V, true);
});

afterEach(() => {
  stop();
  document.body.replaceChildren();
  vi.restoreAllMocks();
});

describe("the talk key", () => {
  it("holds while down and lets go when it comes up, taking both from the page", () => {
    const down = dispatch("keydown", { code: "KeyV" });
    expect(key.held).toBe(true);
    expect(down.defaultPrevented).toBe(true);

    const up = dispatch("keyup", { code: "KeyV" });
    expect(key.held).toBe(false);
    expect(up.defaultPrevented).toBe(true);
    expect(changes).toEqual(["held", "released"]);
  });

  it("ignores auto-repeat, but keeps the repeats from the page while held", () => {
    // A key still down from before, repeating, does not start talking.
    const stale = dispatch("keydown", { code: "KeyV", repeat: true });
    expect(key.held).toBe(false);
    expect(stale.defaultPrevented).toBe(false);

    dispatch("keydown", { code: "KeyV" });
    const repeat = dispatch("keydown", { code: "KeyV", repeat: true });
    expect(repeat.defaultPrevented).toBe(true);
    expect(changes).toEqual(["held"]);
  });

  it("lets go by the key's code whatever the modifiers do meanwhile", () => {
    dispatch("keydown", { code: "KeyV" });
    dispatch("keyup", { code: "KeyV", shiftKey: true });
    expect(key.held).toBe(false);
  });

  it("does not answer a different chord", () => {
    dispatch("keydown", { code: "KeyV", shiftKey: true });
    dispatch("keydown", { code: "KeyB" });
    expect(changes).toEqual([]);
  });

  it.each([
    ["the window losing focus", () => window.dispatchEvent(new Event("blur"))],
    ["the page being put away", () => window.dispatchEvent(new Event("pagehide"))],
    [
      "the page being hidden",
      () => {
        vi.spyOn(document, "visibilityState", "get").mockReturnValue("hidden");
        document.dispatchEvent(new Event("visibilitychange"));
      },
    ],
  ])("is cut by %s", (_what, happen) => {
    dispatch("keydown", { code: "KeyV" });

    happen();

    expect(key.held).toBe(false);
    expect(changes).toEqual(["held", "cut"]);
  });

  it("is not cut by a page that only becomes visible", () => {
    dispatch("keydown", { code: "KeyV" });
    document.dispatchEvent(new Event("visibilitychange"));
    expect(key.held).toBe(true);
  });

  it.each(["input", "textarea", "div"] as const)(
    "leaves a plain key to a focused %s, which types it",
    (tag) => {
      const target = field(tag);

      const down = dispatch("keydown", { code: "KeyV" }, target);

      expect(key.held).toBe(false);
      expect(down.defaultPrevented).toBe(false);
    },
  );

  it("still releases when the key comes up in a field it went down outside", () => {
    dispatch("keydown", { code: "KeyV" });
    dispatch("keyup", { code: "KeyV" }, field());
    expect(key.held).toBe(false);
  });

  it("works from a text field only when the chord has Ctrl, Alt or Meta", () => {
    const target = field("textarea");
    // A single function key types nothing, but is a single key all the same.
    key.configure({ code: "F13", ...none }, true);
    dispatch("keydown", { code: "F13" }, target);
    expect(key.held).toBe(false);

    key.configure({ code: "KeyV", ...none, alt: true }, true);
    const down = dispatch("keydown", { code: "KeyV", altKey: true }, target);

    expect(key.held).toBe(true);
    expect(down.defaultPrevented).toBe(true);
  });

  it("lets go when a modifier of the chord comes up, as macOS sends no key-up under Cmd", () => {
    key.configure(CMD_K, true);
    dispatch("keydown", { code: "KeyK", metaKey: true });

    dispatch("keyup", { code: "MetaLeft" });

    expect(key.held).toBe(false);
    expect(changes).toEqual(["held", "released"]);
  });

  it("is cut when the binding or the mode changes while held, and not by the same chord again", () => {
    dispatch("keydown", { code: "KeyV" });
    key.configure({ ...V }, true);
    expect(key.held).toBe(true);

    key.configure({ code: "KeyB", ...none }, true);
    expect(changes).toEqual(["held", "cut"]);

    dispatch("keydown", { code: "KeyB" });
    key.configure({ code: "KeyB", ...none }, false);
    expect(changes).toEqual(["held", "cut", "held", "cut"]);
  });

  it("does nothing while it may not act, or has no key", () => {
    key.configure(V, false);
    const down = dispatch("keydown", { code: "KeyV" });
    expect(down.defaultPrevented).toBe(false);

    key.configure(null, true);
    dispatch("keydown", { code: "KeyV" });
    expect(changes).toEqual([]);
  });

  it("cuts a held key when its listeners stop", () => {
    dispatch("keydown", { code: "KeyV" });
    stop();
    expect(changes).toEqual(["held", "cut"]);
  });
});
