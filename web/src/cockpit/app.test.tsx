import { render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type * as ApiModule from "./lib/api";

vi.mock("./lib/api", async (importOriginal) => ({
  ...(await importOriginal<typeof ApiModule>()),
  cockpitApi: { bootstrap: vi.fn(() => new Promise(() => {})) },
}));

/** A fresh copy of the panel, as a page load evaluates it. */
async function loadPanel() {
  vi.resetModules();
  return (await import("./app")).CockpitApp;
}

beforeEach(() => {
  localStorage.clear();
});

afterEach(() => {
  localStorage.clear();
  document.documentElement.lang = "";
});

describe("CockpitApp", () => {
  it("draws the panel on its first render, as nothing renders it before the browser", async () => {
    const CockpitApp = await loadPanel();

    render(<CockpitApp><p>Strona</p></CockpitApp>);

    // The gate's status line, while the session is checked.
    expect(screen.getByRole("status")).toBeTruthy();
  });

  it("starts in the browser in the language chosen in the panel's settings", async () => {
    localStorage.setItem("stellarr.cockpit.locale", "pl");
    const CockpitApp = await loadPanel();

    render(<CockpitApp><p>Strona</p></CockpitApp>);

    expect(document.documentElement.lang).toBe("pl");
    expect(screen.getByRole("status").textContent).toBe("Sprawdzanie dostępu do Cockpitu");
  });

  it("starts in English when nothing was chosen", async () => {
    const CockpitApp = await loadPanel();

    render(<CockpitApp><p>Page</p></CockpitApp>);

    expect(document.documentElement.lang).toBe("en");
    expect(screen.getByRole("status").textContent).toBe("Checking access to Cockpit");
  });
});
