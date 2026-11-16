import { render, screen } from "@testing-library/react";
import { renderToString } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("./lib/api", async (importOriginal) => ({
  ...(await importOriginal<typeof import("./lib/api")>()),
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
  it("leaves the panel out of the server's HTML, which cannot know its language", async () => {
    const CockpitApp = await loadPanel();

    expect(renderToString(<CockpitApp><p>Strona</p></CockpitApp>)).toBe("");
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
