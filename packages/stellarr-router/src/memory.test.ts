import { describe, expect, it, vi } from "vitest";

import { createMemoryHistory } from "./memory";

describe("createMemoryHistory", () => {
  it("starts at the address it is given, in canonical form", () => {
    const history = createMemoryHistory("/cockpit/users/?q=a#b");

    expect(history.location).toMatchObject({ pathname: "/cockpit/users", search: "?q=a", hash: "#b" });
    expect(history.href).toBe("/cockpit/users?q=a#b");
    expect(history.navigations).toEqual([]);
  });

  it("reads an address that starts with two slashes as a path on this origin", () => {
    expect(createMemoryHistory("//login").href).toBe("/login");
    expect(createMemoryHistory("//cockpit//users?x=1").href).toBe("/cockpit/users?x=1");
  });

  it("starts at the root without one", () => {
    expect(createMemoryHistory().href).toBe("/");
  });

  it("records each navigation and tells its listeners", () => {
    const history = createMemoryHistory("/");
    const heard = vi.fn();
    history.listen(heard);

    history.push({ pathname: "/login", search: "", hash: "" });
    history.replace({ pathname: "/register", search: "", hash: "" });
    history.leave("/cockpit", false);

    expect(history.navigations).toEqual([
      { kind: "push", href: "/login" },
      { kind: "replace", href: "/register" },
      { kind: "leave", href: "/cockpit", replace: false },
    ]);
    expect(heard.mock.calls).toEqual([["push"], ["replace"]]);
    // A page load is recorded, not followed.
    expect(history.href).toBe("/register");
  });

  it("goes back and forward through its entries", () => {
    const history = createMemoryHistory("/");
    history.push({ pathname: "/login", search: "", hash: "" });
    history.push({ pathname: "/register", search: "", hash: "" });

    history.back();
    expect(history.href).toBe("/login");
    history.back();
    history.back();
    expect(history.href).toBe("/");
    history.forward();
    expect(history.href).toBe("/login");
    expect(history.navigations.filter(({ kind }) => kind === "pop")).toHaveLength(3);
  });

  it("drops the forward entries when it pushes after going back", () => {
    const history = createMemoryHistory("/");
    history.push({ pathname: "/a", search: "", hash: "" });
    history.push({ pathname: "/b", search: "", hash: "" });
    history.back();

    history.push({ pathname: "/c", search: "", hash: "" });

    expect(history.entries).toEqual(["/", "/a", "/c"]);
    expect(history.index).toBe(2);
  });

  it("keeps an entry's key when it is replaced and gives a pushed one a new key", () => {
    const history = createMemoryHistory("/");
    const first = history.location.key;
    history.replace({ pathname: "/login", search: "", hash: "" });
    expect(history.location.key).toBe(first);
    history.push({ pathname: "/register", search: "", hash: "" });
    expect(history.location.key).not.toBe(first);
  });

  it("stops telling a listener that left", () => {
    const history = createMemoryHistory("/");
    const heard = vi.fn();
    history.listen(heard)();

    history.push({ pathname: "/login", search: "", hash: "" });

    expect(heard).not.toHaveBeenCalled();
  });
});
