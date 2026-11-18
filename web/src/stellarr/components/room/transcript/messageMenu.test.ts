import { afterEach, describe, expect, it, vi } from "vitest";

import { messageMenuEntries, type MessageMenuInput } from "./messageMenu";

afterEach(() => vi.restoreAllMocks());

function menuFor(src: string, resolveOriginal?: MessageMenuInput["resolveOriginal"]) {
  return messageMenuEntries({
    messageId: "m1",
    parts: [{ kind: "image", src, alt: "kot.png" }],
    pinned: false,
    pointed: { part: 0, media: 0 },
    own: false,
    canModerate: false,
    developerMode: false,
    confirm: vi.fn(),
    t: (key: string) => key,
    ...(resolveOriginal === undefined ? {} : { resolveOriginal }),
  } as MessageMenuInput);
}

describe("messageMenuEntries for an image", () => {
  it("fetches an encrypted original before it is downloaded", async () => {
    const saved: string[] = [];
    vi.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(function (this: HTMLAnchorElement) {
      saved.push(this.href);
    });
    const resolveOriginal = vi.fn(async (_src: string) => "blob:original");

    const download = menuFor("e2ee:u1", resolveOriginal).find(
      (entry) => typeof entry === "object" && entry.id === "download",
    );
    if (typeof download !== "object" || !("onSelect" in download)) throw new Error("no download entry");
    download.onSelect();

    await vi.waitFor(() => expect(saved).toEqual(["blob:original"]));
    expect(resolveOriginal).toHaveBeenCalledWith("e2ee:u1");
  });

  it("offers no address to copy for an original that is fetched on demand", () => {
    const ids = (src: string) =>
      menuFor(src).flatMap((entry) => (typeof entry === "object" ? [entry.id] : []));

    expect(ids("e2ee:u1")).not.toContain("copy-image-link");
    expect(ids("/assets/a")).toContain("copy-image-link");
  });

  it("does not ask for an address that is already usable", () => {
    vi.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(() => {});
    const resolveOriginal = vi.fn(async (_src: string) => "unused");

    const download = menuFor("/assets/a", resolveOriginal).find(
      (entry) => typeof entry === "object" && entry.id === "download",
    );
    if (typeof download !== "object" || !("onSelect" in download)) throw new Error("no download entry");
    download.onSelect();

    expect(resolveOriginal).not.toHaveBeenCalled();
  });
});
