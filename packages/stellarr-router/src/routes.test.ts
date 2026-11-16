import { describe, expect, it } from "vitest";

import { defineRoutes } from "./routes";

const Home = () => null;
const Login = () => null;
const Invite = () => null;
const Fixed = () => null;
const NotFound = () => null;

const table = defineRoutes({
  routes: [
    { path: "/", page: Home },
    { path: "/login", page: Login },
    { path: "/invite/new", page: Fixed },
    { path: "/invite/:code", page: Invite },
  ],
  notFound: NotFound,
});

describe("defineRoutes", () => {
  it("finds the page for an address", () => {
    expect(table.match("/")).toEqual({ page: Home, params: {}, route: "/" });
    expect(table.match("/login")).toEqual({ page: Login, params: {}, route: "/login" });
  });

  it("reads a parameter, decoded", () => {
    expect(table.match("/invite/abc123").params).toEqual({ code: "abc123" });
    expect(table.match(`/invite/${encodeURIComponent("a/b c")}`).params).toEqual({ code: "a/b c" });
  });

  it("matches a parameter whose escapes are malformed, with no value", () => {
    expect(table.match("/invite/%E0%A4%A")).toEqual({ page: Invite, params: { code: null }, route: "/invite/:code" });
  });

  it("answers an address with a trailing slash as the address without it", () => {
    expect(table.match("/login/").page).toBe(Login);
    expect(table.match("/invite/abc/").params).toEqual({ code: "abc" });
  });

  it("compares fixed segments decoded, as the browser shows them", () => {
    expect(table.match("/log%69n").page).toBe(Login);
  });

  it("takes the first route that matches, so a fixed address can sit before a parameter", () => {
    expect(table.match("/invite/new").page).toBe(Fixed);
  });

  it("gives every other address the not-found page", () => {
    for (const path of ["/nope", "/invite", "/invite//x", "/invite/a/b", "/login/more", "/cockpit"]) {
      expect(table.match(path), path).toEqual({ page: NotFound, params: {}, route: null });
    }
  });

  it("claims only what a route matches, never the not-found page's addresses", () => {
    expect(table.claims("/")).toBe(true);
    expect(table.claims("/invite/abc")).toBe(true);
    expect(table.claims("/login/")).toBe(true);
    expect(table.claims("/cockpit")).toBe(false);
    expect(table.claims("/nope")).toBe(false);
  });

  it("refuses a path it would never match", () => {
    for (const path of ["login", "/login/", "/a//b", "/x/:", "/x/:a/:a", "/x/:1"]) {
      expect(() => defineRoutes({ routes: [{ path, page: Home }], notFound: NotFound }), path).toThrow();
    }
  });

  it("refuses two routes for the same addresses", () => {
    expect(() =>
      defineRoutes({
        routes: [
          { path: "/invite/:code", page: Invite },
          { path: "/invite/:id", page: Invite },
        ],
        notFound: NotFound,
      }),
    ).toThrow(/same addresses/);
  });
});
