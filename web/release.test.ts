import { describe, expect, it } from "vitest";

import { RELEASE_META as CLIENT_META, RELEASE_PATH as CLIENT_PATH } from "./src/stellarr/lib/platform/release";
import { releaseFile, releaseId, RELEASE_META, RELEASE_PATH, stampDocument } from "./release";

const bytes = (text: string) => new TextEncoder().encode(text);
const files = (entries: Record<string, string>) => new Map(Object.entries(entries).map(([path, text]) => [path, bytes(text)]));

describe("the release id", () => {
  const base = { "index.html": "<html></html>", "static/web-1.js": "a", "audio/worklet.js": "b" };

  it("is the same for the same files in any order", () => {
    const reversed = Object.fromEntries(Object.entries(base).reverse());
    expect(releaseId(files(reversed))).toBe(releaseId(files(base)));
  });

  it("changes when a byte of any file changes", () => {
    for (const path of Object.keys(base)) {
      expect(releaseId(files({ ...base, [path]: "changed" }))).not.toBe(releaseId(files(base)));
    }
  });

  it("changes when a file is renamed, added or removed", () => {
    const id = releaseId(files(base));
    expect(releaseId(files({ "index.html": base["index.html"], "static/web-2.js": "a", "audio/worklet.js": "b" }))).not.toBe(id);
    expect(releaseId(files({ ...base, "robots.txt": "" }))).not.toBe(id);
    expect(releaseId(files({ "index.html": base["index.html"], "static/web-1.js": "a" }))).not.toBe(id);
  });

  it("does not let one file's end pass for the start of the next", () => {
    expect(releaseId(files({ a: "xy", b: "z" }))).not.toBe(releaseId(files({ a: "x", b: "yz" })));
  });
});

describe("what the build writes the id into", () => {
  it("puts it in the head of the document, where the page reads it", () => {
    const stamped = new DOMParser().parseFromString(
      stampDocument("<html><head><title>x</title></head><body></body></html>", "abc"),
      "text/html",
    );
    expect(stamped.querySelector(`meta[name="${RELEASE_META}"]`)?.getAttribute("content")).toBe("abc");
  });

  it("refuses a document with no head", () => {
    expect(() => stampDocument("<p>no head</p>", "abc")).toThrow();
  });

  it("writes the id into the file the page asks for", () => {
    expect(JSON.parse(releaseFile("abc"))).toEqual({ id: "abc" });
  });
});

describe("the page's side of it", () => {
  it("asks for the file and the meta the build writes", () => {
    expect(CLIENT_PATH).toBe(RELEASE_PATH);
    expect(CLIENT_META).toBe(RELEASE_META);
  });
});
