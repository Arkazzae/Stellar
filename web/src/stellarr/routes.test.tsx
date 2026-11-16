import { describe, expect, it } from "vitest";

import ConfirmEmailPage from "./pages/ConfirmEmailPage";
import HomePage from "./pages/HomePage";
import InvitePage from "./pages/InvitePage";
import LoginPage from "./pages/LoginPage";
import NotFoundPage from "./pages/NotFoundPage";
import RegisterPage from "./pages/RegisterPage";
import { routes } from "./routes";

describe("the web client's addresses", () => {
  it("give each page at its address", () => {
    expect(routes.match("/").page).toBe(HomePage);
    expect(routes.match("/login").page).toBe(LoginPage);
    expect(routes.match("/confirm-email").page).toBe(ConfirmEmailPage);
    expect(routes.match("/invite/abc123")).toMatchObject({ page: InvitePage, params: { code: "abc123" } });
  });

  // nginx merges the slashes and answers such an address with this document.
  it("give the page an address names when it starts with a run of slashes", () => {
    expect(routes.match("//").page).toBe(HomePage);
    expect(routes.match("//login").page).toBe(LoginPage);
    expect(routes.match("///register/").page).toBe(RegisterPage);
    expect(routes.match("//invite//abc123")).toMatchObject({ page: InvitePage, params: { code: "abc123" } });
    expect(routes.match("//nope").page).toBe(NotFoundPage);
  });

  it("leave Cockpit's addresses to Cockpit's document", () => {
    expect(routes.claims("/cockpit")).toBe(false);
    expect(routes.claims("//cockpit//users")).toBe(false);
  });
});
