import { describe, expect, it } from "vitest";

import { DashboardPage } from "./pages/dashboard-page";
import { NotFoundPage } from "./pages/not-found-page";
import { UsersPage } from "./pages/users-page";
import { routes } from "./routes";

describe("Cockpit's addresses", () => {
  it("give each page at its address", () => {
    expect(routes.match("/cockpit").page).toBe(DashboardPage);
    expect(routes.match("/cockpit/users").page).toBe(UsersPage);
    expect(routes.match("/cockpit/nope").page).toBe(NotFoundPage);
  });

  // nginx merges the slashes and answers such an address with this document.
  it("give the page an address names when it has runs of slashes", () => {
    expect(routes.match("//cockpit").page).toBe(DashboardPage);
    expect(routes.match("//cockpit//users").page).toBe(UsersPage);
    expect(routes.match("/cockpit//users/").page).toBe(UsersPage);
  });

  it("leave the web client's addresses to its document", () => {
    expect(routes.claims("/")).toBe(false);
    expect(routes.claims("//login")).toBe(false);
  });
});
