import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { AppShell } from "./app-shell";
import { CockpitContext, type CockpitContextValue } from "../state/cockpit-context";
import { selectPolish } from "../test/locale";
import { cockpitValue } from "../test/cockpit-fixtures";
import { visit } from "../test/navigation";

vi.mock("next/navigation", () => import("../test/navigation"));
vi.mock("next/link", () => import("../test/navigation").then(({ Link }) => ({ default: Link })));

function renderShell(path: string, value: CockpitContextValue = cockpitValue()) {
  visit(path);
  return render(
    <CockpitContext.Provider value={value}>
      <AppShell>
        <p>Treść strony</p>
      </AppShell>
    </CockpitContext.Provider>,
  );
}

/** jsdom applies no style sheet, so the fixed sidebar is always there; the first navigation is its. */
function sidebarNavigation() {
  const [navigation] = screen.getAllByRole("navigation", { name: "Nawigacja Cockpitu" });
  if (!navigation) throw new Error("no navigation rendered");
  return navigation;
}

function activeLinks() {
  return within(sidebarNavigation())
    .getAllByRole("link")
    .filter((link) => link.getAttribute("aria-current") === "page")
    .map((link) => link.textContent);
}

selectPolish();

describe("AppShell navigation", () => {
  it("lists the eight areas of the panel in order", () => {
    renderShell("/cockpit");

    const labels = within(sidebarNavigation())
      .getAllByRole("link")
      .map((link) => link.textContent);
    expect(labels).toEqual([
      "Przegląd",
      "Użytkownicy",
      "Serwery",
      "Runtime",
      "Logi",
      "Backup",
      "Integracje",
      "Ustawienia",
    ]);
  });

  it("links each area to its page under /cockpit", () => {
    renderShell("/cockpit");

    const addresses = within(sidebarNavigation())
      .getAllByRole("link")
      .map((link) => link.getAttribute("href"));
    expect(addresses).toEqual([
      "/cockpit",
      "/cockpit/users",
      "/cockpit/servers",
      "/cockpit/runtime",
      "/cockpit/logs",
      "/cockpit/backups",
      "/cockpit/integrations",
      "/cockpit/settings",
    ]);
  });

  it("renders the routed page", () => {
    renderShell("/cockpit/users");

    expect(screen.getByRole("main").textContent).toBe("Treść strony");
  });

  it.each([
    ["/cockpit", "Przegląd"],
    ["/cockpit/users", "Użytkownicy"],
    ["/cockpit/users/user-1", "Użytkownicy"],
    ["/cockpit/servers", "Serwery"],
    ["/cockpit/runtime", "Runtime"],
    ["/cockpit/logs", "Logi"],
    ["/cockpit/backups", "Backup"],
    ["/cockpit/integrations", "Integracje"],
    ["/cockpit/settings", "Ustawienia"],
  ])("marks the item for %s as the only active one and names it in the header", (path, label) => {
    renderShell(path);

    expect(activeLinks()).toEqual([label]);
    expect(within(screen.getByRole("banner")).getByText(label)).toBeTruthy();
  });

  it("matches the root exactly, so an unknown page marks nothing and keeps the product name", () => {
    renderShell("/cockpit/missing");

    expect(activeLinks()).toEqual([]);
    expect(within(screen.getByRole("banner")).getByText("Stellarr Cockpit")).toBeTruthy();
  });

  it("does not take a page whose name only starts like an area's for that area", () => {
    renderShell("/cockpit/usersettings");

    expect(activeLinks()).toEqual([]);
    expect(within(screen.getByRole("banner")).getByText("Stellarr Cockpit")).toBeTruthy();
  });

  it("shows the instance name in the header", () => {
    renderShell("/cockpit");

    expect(within(screen.getByRole("banner")).getByText("Domek")).toBeTruthy();
  });
});

describe("AppShell mobile navigation", () => {
  it("opens the same navigation in a sheet and closes it once a link is followed", async () => {
    const user = userEvent.setup();
    renderShell("/cockpit");

    await user.click(screen.getByRole("button", { name: "Otwórz nawigację" }));
    const sheet = await screen.findByRole("dialog");
    const sheetLinks = within(sheet).getAllByRole("link");
    expect(sheetLinks).toHaveLength(8);

    await user.click(within(sheet).getByRole("link", { name: "Runtime" }));

    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(within(screen.getByRole("banner")).getByText("Runtime")).toBeTruthy();
  });
});

describe("AppShell operator menu", () => {
  it("shows who is signed in", async () => {
    const user = userEvent.setup();
    renderShell("/cockpit");

    await user.click(screen.getByRole("button", { name: /Ola/ }));

    const menu = await screen.findByRole("menu");
    expect(within(menu).getByText("Ola Owner")).toBeTruthy();
    expect(within(menu).getByText("@owner")).toBeTruthy();
  });

  it("leaves the panel for the app with a full page load", async () => {
    const user = userEvent.setup();
    renderShell("/cockpit/users");

    await user.click(screen.getByRole("button", { name: /Ola/ }));

    const toApp = await screen.findByRole("menuitem", { name: "Przejdź do aplikacji" });
    // The app has a root layout of its own, so it is a plain link and not the router's.
    expect(toApp.getAttribute("href")).toBe("/");
  });

  it("logs out through the session", async () => {
    const user = userEvent.setup();
    const value = cockpitValue();
    renderShell("/cockpit", value);

    await user.click(screen.getByRole("button", { name: /Ola/ }));
    await user.click(await screen.findByRole("menuitem", { name: "Wyloguj się" }));

    expect(value.logout).toHaveBeenCalledOnce();
  });
});
