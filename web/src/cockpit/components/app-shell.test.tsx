import { Router } from "@stellarr/router";
import { createMemoryHistory, type MemoryHistory } from "@stellarr/router/testing";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";

import { AppShell } from "./app-shell";
import { CockpitContext, type CockpitContextValue } from "../state/cockpit-context";
import { selectPolish } from "../test/locale";
import { cockpitValue } from "../test/cockpit-fixtures";
import { routes } from "../routes";

let history: MemoryHistory;

function renderShell(path: string, value: CockpitContextValue = cockpitValue()) {
  history = createMemoryHistory(path);
  return render(
    <Router routes={routes} history={history}>
      <CockpitContext.Provider value={value}>
        <AppShell>
          <p>Treść strony</p>
        </AppShell>
      </CockpitContext.Provider>
    </Router>,
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
  it("lists the nine areas of the panel in order", () => {
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
      "Aktywności",
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
      "/cockpit/activities",
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
    ["/cockpit/activities", "Aktywności"],
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
    expect(sheetLinks).toHaveLength(9);

    await user.click(within(sheet).getByRole("link", { name: "Runtime" }));

    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(within(screen.getByRole("banner")).getByText("Runtime")).toBeTruthy();
    expect(history.navigations).toEqual([{ kind: "push", href: "/cockpit/runtime" }]);
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

  it("leads out of the panel to the app", async () => {
    const user = userEvent.setup();
    renderShell("/cockpit/users");

    await user.click(screen.getByRole("button", { name: /Ola/ }));

    const toApp = await screen.findByRole("menuitem", { name: "Przejdź do aplikacji" });
    // The app's root, not the panel's: the link is not under /cockpit, so
    // following it loads the web client's document.
    expect(toApp.getAttribute("href")).toBe("/");
    await user.click(toApp);
    expect(history.navigations).toEqual([{ kind: "leave", href: "/", replace: false }]);
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
