import { describe, expect, it } from "vitest";

import { RELEASE_META as CLIENT_META, RELEASE_PATH as CLIENT_PATH } from "./src/stellarr/lib/platform/release";
import {
  releaseFile,
  releaseId,
  RELEASE_META,
  RELEASE_PATH,
  shellFiles,
  stampDocument,
  type ChunkLinks,
} from "./release";

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

  it("writes the manifest into the file the page and the worker ask for", () => {
    const manifest = { id: "abc", shell: ["/index.html"], directories: ["static"] };
    expect(JSON.parse(releaseFile(manifest))).toEqual(manifest);
  });
});

describe("the files of the web client's shell", () => {
  const links = (imports: string[] = [], dynamicImports: string[] = [], css: string[] = []): ChunkLinks => ({ imports, dynamicImports, css });
  const chunks = new Map([
    ["static/web.js", links(["static/shared.js"], ["static/app.js"], ["static/web.css"])],
    ["static/shared.js", links()],
    ["static/app.js", links(["static/shared.js"])],
    ["static/cockpit.js", links(["static/shared.js"], [], ["static/cockpit.css"])],
  ]);
  const texts: Record<string, string> = {
    "index.html": '<script src="/static/web.js"></script><link href="/static/icon.svg">',
    "static/web.js": "",
    "static/shared.js": "",
    "static/app.js": 'import("/static/mascot.png"); fetch("/audio/detector-worklet.js"); "/static/gone.svg"',
    "static/web.css": "src: url(/fonts/outfit.woff2)",
    "static/cockpit.js": "",
    "static/cockpit.css": "src: url(/static/cockpit-only.woff2)",
    "static/icon.svg": "",
    "static/mascot.png": "",
    "fonts/outfit.woff2": "",
    "audio/detector-worklet.js": "",
    "static/cockpit-only.woff2": "",
  };
  const read = (file: string) => texts[file] ?? null;

  it("holds the document, what it loads, on demand or not, and what those name", () => {
    expect(shellFiles(chunks, read)).toEqual([
      "/index.html",
      "/fonts/outfit.woff2",
      "/static/app.js",
      "/static/icon.svg",
      "/static/mascot.png",
      "/static/shared.js",
      "/static/web.css",
      "/static/web.js",
    ]);
  });

  it("leaves out Cockpit's files, other folders and names with no file", () => {
    const shell = shellFiles(chunks, read);
    expect(shell).not.toContain("/static/cockpit.js");
    expect(shell).not.toContain("/static/cockpit-only.woff2");
    expect(shell).not.toContain("/audio/detector-worklet.js");
    expect(shell).not.toContain("/static/gone.svg");
  });
});

describe("the page's side of it", () => {
  it("asks for the file and the meta the build writes", () => {
    expect(CLIENT_PATH).toBe(RELEASE_PATH);
    expect(CLIENT_META).toBe(RELEASE_META);
  });
});
