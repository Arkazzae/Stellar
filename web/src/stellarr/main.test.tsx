import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type * as I18n from "./i18n";

/**
 * The entry renders once the page's catalog is here, and in its place a
 * notice that needs no catalog when it cannot be fetched.
 */

const control = vi.hoisted(() => ({
  load: (_locale: string): Promise<void> => Promise.resolve(),
}));

vi.mock("./i18n", async (importOriginal) => ({
  ...(await importOriginal<typeof I18n>()),
  loadCatalog: (locale: string) => control.load(locale),
}));

/** Runs the entry on a fresh page whose pre-paint script chose `locale`. */
async function start(locale: "pl" | "en") {
  document.documentElement.lang = locale;
  document.body.innerHTML = '<div id="root"></div>';
  vi.resetModules();
  await import("./main");
}

beforeEach(() => {
  control.load = () => Promise.resolve();
});

afterEach(() => {
  document.body.innerHTML = "";
});

describe("the web client's entry", () => {
  it("asks for the catalog of the page's language and renders nothing before it is here", async () => {
    const asked: string[] = [];
    control.load = (locale) => {
      asked.push(locale);
      return new Promise(() => undefined);
    };

    await start("en");

    expect(asked).toEqual(["en"]);
    expect(document.getElementById("root")?.childElementCount).toBe(0);
  });

  it("explains a catalog that cannot be fetched, in the page's language, and loads the page again", async () => {
    control.load = () => Promise.reject(new Error("Failed to fetch dynamically imported module"));
    const reload = vi.fn();
    vi.spyOn(window, "location", "get").mockReturnValue({ ...window.location, reload });

    await start("pl");

    expect(await screen.findByText("Nie udało się wczytać Stellarr")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Wczytaj ponownie" }));
    expect(reload).toHaveBeenCalledOnce();
  });
});
