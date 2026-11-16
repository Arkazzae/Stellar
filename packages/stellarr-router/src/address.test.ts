import { describe, expect, it } from "vitest";

import { canonicalPath, entryKey, formatAddress, resolveAddress, sameAddress } from "./address";

const ORIGIN = "https://chat.example.test";
const AT_LOGIN = { pathname: "/login", search: "?returnUrl=%2F", hash: "" };

describe("canonicalPath", () => {
  it("drops a trailing slash, however many", () => {
    expect(canonicalPath("/login/")).toBe("/login");
    expect(canonicalPath("/cockpit//")).toBe("/cockpit");
    expect(canonicalPath("/invite/abc")).toBe("/invite/abc");
  });

  it("keeps the root", () => {
    expect(canonicalPath("/")).toBe("/");
    expect(canonicalPath("//")).toBe("/");
    expect(canonicalPath("")).toBe("/");
  });

  it("merges every run of slashes into one, as nginx does before it answers", () => {
    expect(canonicalPath("/a//b")).toBe("/a/b");
    expect(canonicalPath("//login")).toBe("/login");
    expect(canonicalPath("///register")).toBe("/register");
    expect(canonicalPath("//cockpit//users//")).toBe("/cockpit/users");
    expect(canonicalPath("//invite//abc123")).toBe("/invite/abc123");
  });

  it("never yields a path a browser would read as another host", () => {
    for (const path of ["//login", "///evil.example.test/x", "//", "/\u0000//x"]) {
      expect(canonicalPath(path).startsWith("//"), path).toBe(false);
    }
  });

  // A pattern anchored only at the end retried every slash of a run that does
  // not end the path: 40,000 of them took over a second, and this many would
  // take minutes and freeze the tab. Anyone can put such a run in a link.
  it("takes time linear in the length of a long run of slashes, wherever it is", () => {
    const run = "/".repeat(300_000);
    const started = performance.now();

    expect(canonicalPath(`${run}x`)).toBe("/x");
    expect(canonicalPath(`/a${run}b`)).toBe("/a/b");
    expect(canonicalPath(`/a${run}`)).toBe("/a");
    expect(canonicalPath(run)).toBe("/");
    expect(resolveAddress(`${run}x`, AT_LOGIN, ORIGIN).sameOrigin).toBe(false);
    expect(resolveAddress(`/a${run}`, AT_LOGIN, ORIGIN).pathname).toBe("/a");

    expect(performance.now() - started).toBeLessThan(250);
  });
});

describe("resolveAddress", () => {
  it("reads a rooted path on this origin, keeping the query and the fragment", () => {
    expect(resolveAddress("/register?invite=1#top", AT_LOGIN, ORIGIN)).toEqual({
      sameOrigin: true,
      href: "/register?invite=1#top",
      pathname: "/register",
      search: "?invite=1",
      hash: "#top",
    });
  });

  it("makes the path canonical", () => {
    expect(resolveAddress("/cockpit/", AT_LOGIN, ORIGIN)).toMatchObject({ href: "/cockpit", pathname: "/cockpit" });
  });

  it("resolves a relative address against the current one, as the browser would", () => {
    expect(resolveAddress("forgot-password", AT_LOGIN, ORIGIN).pathname).toBe("/forgot-password");
    expect(resolveAddress("?fresh=1", AT_LOGIN, ORIGIN).href).toBe("/login?fresh=1");
    expect(resolveAddress("#code", AT_LOGIN, ORIGIN).href).toBe("/login?returnUrl=%2F#code");
  });

  it("takes a whole URL on this origin as a path", () => {
    expect(resolveAddress(`${ORIGIN}/login`, AT_LOGIN, ORIGIN)).toMatchObject({ sameOrigin: true, href: "/login" });
  });

  it("leaves another origin, another scheme and a protocol-relative address whole", () => {
    expect(resolveAddress("https://docs.example.test/x/", AT_LOGIN, ORIGIN)).toMatchObject({
      sameOrigin: false,
      href: "https://docs.example.test/x/",
    });
    expect(resolveAddress("//evil.example.test/login", AT_LOGIN, ORIGIN).sameOrigin).toBe(false);
    expect(resolveAddress("mailto:owner@example.test", AT_LOGIN, ORIGIN).sameOrigin).toBe(false);
  });
});

describe("sameAddress", () => {
  it("compares the path, the query and the fragment, not the entry", () => {
    expect(sameAddress({ ...AT_LOGIN }, { ...AT_LOGIN })).toBe(true);
    expect(sameAddress(AT_LOGIN, { ...AT_LOGIN, search: "" })).toBe(false);
    expect(sameAddress(AT_LOGIN, { ...AT_LOGIN, hash: "#x" })).toBe(false);
  });
});

describe("formatAddress and entryKey", () => {
  it("write the address without the origin", () => {
    expect(formatAddress(AT_LOGIN)).toBe("/login?returnUrl=%2F");
  });

  it("name every entry differently", () => {
    expect(new Set(Array.from({ length: 50 }, entryKey)).size).toBe(50);
  });
});
