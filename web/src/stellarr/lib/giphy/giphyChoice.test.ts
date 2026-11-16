import { beforeEach, describe, expect, it, vi } from "vitest";

import { createGiphyChoice, GIPHY_CHOICE_KEY } from "./giphyChoice";

beforeEach(() => {
  window.localStorage.clear();
});

describe("the GIPHY switch", () => {
  it("is on for someone who never touched it, once the account has answered", () => {
    const choice = createGiphyChoice();

    expect(choice.isEnabled()).toBe(true);
    expect(choice.access()).toBe("waiting");

    choice.accountRead();
    expect(choice.access()).toBe("on");
  });

  it("is off at once when this browser has it off, without waiting for the account", () => {
    window.localStorage.setItem(GIPHY_CHOICE_KEY, JSON.stringify({ enabled: false }));
    const choice = createGiphyChoice();

    expect(choice.access()).toBe("off");
  });

  it("keeps the person's choice in this browser and tells whoever listens", () => {
    const choice = createGiphyChoice();
    choice.accountRead();
    const listener = vi.fn();
    choice.subscribe(listener);

    choice.set(false);

    expect(choice.access()).toBe("off");
    expect(JSON.parse(window.localStorage.getItem(GIPHY_CHOICE_KEY) ?? "null")).toEqual({ enabled: false });
    expect(listener).toHaveBeenCalledOnce();
    expect(createGiphyChoice().isEnabled()).toBe(false);
  });

  it("waits again when another account signs in", () => {
    const choice = createGiphyChoice();
    choice.accountRead();

    choice.awaitAccount();

    expect(choice.access()).toBe("waiting");
  });

  it("follows a change made in another tab", () => {
    const choice = createGiphyChoice();
    choice.accountRead();
    const listener = vi.fn();
    const unsubscribe = choice.subscribe(listener);

    window.localStorage.setItem(GIPHY_CHOICE_KEY, JSON.stringify({ enabled: false }));
    window.dispatchEvent(new StorageEvent("storage", { key: GIPHY_CHOICE_KEY }));

    expect(choice.access()).toBe("off");
    expect(listener).toHaveBeenCalled();
    unsubscribe();
  });

  it("reads an unreadable stored value as on", () => {
    window.localStorage.setItem(GIPHY_CHOICE_KEY, "{not json");

    expect(createGiphyChoice().isEnabled()).toBe(true);
  });
});
