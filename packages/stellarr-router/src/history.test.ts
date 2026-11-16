import { waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { createBrowserHistory } from "./history";

function at(href: string): void {
  window.history.replaceState(null, "", href);
}

beforeEach(() => {
  at("/");
});

describe("the browser's history", () => {
  it("rewrites the address it was loaded at to the canonical path, keeping the query", () => {
    at("/login/?returnUrl=%2Fcockpit#top");

    const history = createBrowserHistory(window);

    expect(window.location.pathname).toBe("/login");
    expect(window.location.search).toBe("?returnUrl=%2Fcockpit");
    expect(window.location.hash).toBe("#top");
    expect(history.location).toMatchObject({ pathname: "/login", search: "?returnUrl=%2Fcockpit", hash: "#top" });
  });

  it("names the entry it was loaded at and keeps the name across a reload", () => {
    at("/register");
    const first = createBrowserHistory(window).location.key;

    expect(first).not.toBe("");
    expect(createBrowserHistory(window).location.key).toBe(first);
  });

  it("takes scroll restoration over from the browser", () => {
    window.history.scrollRestoration = "auto";

    createBrowserHistory(window);

    expect(window.history.scrollRestoration).toBe("manual");
  });

  it("pushes and replaces entries of the window's history", () => {
    const history = createBrowserHistory(window);
    const heard = vi.fn();
    history.listen(heard);
    const length = window.history.length;

    history.push({ pathname: "/login", search: "?fresh=1", hash: "" });
    expect(window.location.pathname + window.location.search).toBe("/login?fresh=1");
    expect(window.history.length).toBe(length + 1);
    const pushed = history.location.key;

    history.replace({ pathname: "/register", search: "", hash: "" });
    expect(window.location.pathname).toBe("/register");
    expect(window.history.length).toBe(length + 1);
    expect(history.location.key).toBe(pushed);

    expect(heard.mock.calls).toEqual([["push"], ["replace"]]);
  });

  it("follows Back and Forward", async () => {
    const history = createBrowserHistory(window);
    const heard = vi.fn();
    history.listen(heard);
    const start = history.location.key;
    history.push({ pathname: "/login", search: "", hash: "" });

    window.history.back();

    await waitFor(() => expect(history.location.pathname).toBe("/"));
    expect(history.location.key).toBe(start);
    expect(heard).toHaveBeenLastCalledWith("pop");
  });

  it("loads another document to leave the app, replacing the entry when asked", () => {
    const assign = vi.fn();
    const replace = vi.fn();
    const fake = {
      location: { pathname: "/", search: "", hash: "", origin: "https://chat.example.test", assign, replace },
      history: { state: { stellarrRouterKey: "k" }, replaceState: vi.fn(), pushState: vi.fn(), scrollRestoration: "auto" },
      addEventListener: vi.fn(),
    } as unknown as Window;
    const history = createBrowserHistory(fake);

    history.leave("/cockpit", false);
    history.leave("https://docs.example.test/", true);

    expect(assign).toHaveBeenCalledExactlyOnceWith("/cockpit");
    expect(replace).toHaveBeenCalledExactlyOnceWith("https://docs.example.test/");
    expect(history.origin).toBe("https://chat.example.test");
    expect(history.native).toBe(true);
  });
});
