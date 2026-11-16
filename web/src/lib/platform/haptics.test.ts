import { screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi, type Mock } from "vitest";

/** The labels the module added to the page; there should never be more than one. */
function switchLabels(): HTMLLabelElement[] {
  return [...document.body.querySelectorAll("label")];
}

/** Counts how often the switch was really toggled, not merely clicked. */
function countToggles(label: HTMLLabelElement): () => number {
  let toggles = 0;
  label.querySelector("input")?.addEventListener("change", () => toggles++);
  return () => toggles;
}

beforeEach(() => {
  // The switch is module state, so every test starts from a fresh module.
  vi.resetModules();
});

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  // An own `body` shadows the document's getter while a test removes it.
  Reflect.deleteProperty(document, "body");
  document.body.replaceChildren();
});

describe("triggerHaptic with the Vibration API", () => {
  let vibrate: Mock<(pattern: VibratePattern) => boolean>;

  beforeEach(() => {
    vibrate = vi.fn<(pattern: VibratePattern) => boolean>(() => true);
    vi.stubGlobal("navigator", { vibrate });
  });

  it("plays each cue as its pulse at partial strength", async () => {
    const { triggerHaptic } = await import("./haptics");

    triggerHaptic("selection");
    triggerHaptic("light");

    expect(vibrate.mock.calls).toEqual([[[2, 6]], [[6, 9]]]);
  });

  it("adds nothing to the page", async () => {
    const { triggerHaptic } = await import("./haptics");

    triggerHaptic("light");

    expect(switchLabels()).toHaveLength(0);
  });

  it("swallows a pattern the browser refuses", async () => {
    vibrate.mockImplementation(() => {
      throw new TypeError("vibration blocked");
    });
    const { triggerHaptic } = await import("./haptics");

    expect(() => triggerHaptic("light")).not.toThrow();
  });

  it("stays inert on the server", async () => {
    const { triggerHaptic } = await import("./haptics");
    vi.stubGlobal("window", undefined);

    triggerHaptic("light");

    expect(vibrate).not.toHaveBeenCalled();
  });
});

describe("triggerHaptic without the Vibration API", () => {
  // jsdom has no `navigator.vibrate`, which is the case on Safari for iOS.

  it("creates nothing until the first cue", async () => {
    await import("./haptics");

    expect(switchLabels()).toHaveLength(0);
  });

  it("toggles one switch shared by every cue", async () => {
    const { triggerHaptic } = await import("./haptics");

    triggerHaptic("selection");
    const [label] = switchLabels();
    expect(label).toBeDefined();
    // The cue that makes the switch also plays.
    expect(label!.querySelector("input")?.checked).toBe(true);
    const toggles = countToggles(label!);
    triggerHaptic("light");
    triggerHaptic("selection");

    expect(switchLabels()).toEqual([label]);
    expect(toggles()).toBe(2);
    const input = label!.querySelector("input");
    expect(input?.type).toBe("checkbox");
    expect(input?.hasAttribute("switch")).toBe(true);
  });

  it("keeps the switch out of sight, speech and the tab order", async () => {
    const { triggerHaptic } = await import("./haptics");

    triggerHaptic("selection");

    const [label] = switchLabels();
    expect(label?.style.display).toBe("none");
    expect(label?.textContent).toBe("");
    // The switch is in the page, but not in what assistive technology reads.
    expect(screen.queryByRole("checkbox", { hidden: true })).not.toBeNull();
    expect(screen.queryByRole("checkbox")).toBeNull();
    expect(screen.queryByRole("switch")).toBeNull();
  });

  it("waits for the body instead of throwing", async () => {
    const { triggerHaptic } = await import("./haptics");
    Object.defineProperty(document, "body", { configurable: true, get: () => null });
    const createElement = vi.spyOn(document, "createElement");

    expect(() => triggerHaptic("light")).not.toThrow();
    expect(createElement).not.toHaveBeenCalled();

    Reflect.deleteProperty(document, "body");
    expect(switchLabels()).toHaveLength(0);
    triggerHaptic("light");
    expect(switchLabels()).toHaveLength(1);
  });

  it("puts the same switch back when a render removed it", async () => {
    const { triggerHaptic } = await import("./haptics");

    triggerHaptic("selection");
    const [label] = switchLabels();
    document.body.replaceChildren();
    const toggles = countToggles(label!);
    triggerHaptic("selection");

    expect(switchLabels()).toEqual([label]);
    expect(toggles()).toBe(1);
  });

  it("stays inert on the server", async () => {
    const { triggerHaptic } = await import("./haptics");
    vi.stubGlobal("window", undefined);

    triggerHaptic("light");

    expect(switchLabels()).toHaveLength(0);
  });
});
