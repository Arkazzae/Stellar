import { act, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type * as ProviderModule from "./LanguageProvider";

/**
 * Switching to a language whose catalog this page has not fetched yet. Each
 * test starts from a registry holding the English catalog alone, as a page in
 * English has, and controls when the Polish one arrives.
 */

const STORAGE_KEY = "stellarr_language_settings";

let release: () => void = () => undefined;

/** The Polish catalog arrives when `release` is called. */
function holdPolish() {
  const gate = new Promise<void>((resolve) => {
    release = resolve;
  });
  vi.doMock("@/i18n/messages/pl", async () => {
    await gate;
    return await vi.importActual<object>("@/i18n/messages/pl");
  });
}

/** The Polish catalog cannot be fetched, as after an upgrade replaced its file. */
function losePolish() {
  vi.doMock("@/i18n/messages/pl", () => {
    throw new Error("Failed to fetch dynamically imported module");
  });
}

let provider: typeof ProviderModule;

async function englishPage() {
  vi.resetModules();
  const i18n = await import("@/i18n");
  await i18n.loadCatalog("en");
  provider = await import("./LanguageProvider");
}

function Probe() {
  const language = provider.useLanguage();
  return (
    <div>
      <output aria-label="locale">{language.locale}</output>
      <output aria-label="switching">{JSON.stringify(language.switching)}</output>
      <output aria-label="message">{language.t("settings.language.title")}</output>
      <button type="button" onClick={() => language.update({ locale: "pl" })}>
        polski
      </button>
      <button type="button" onClick={() => language.update({ locale: "en" })}>
        english
      </button>
      <button type="button" onClick={() => language.update({ chatTranslationEnabled: true, locale: "pl" })}>
        both
      </button>
    </div>
  );
}

async function mount() {
  const { LanguageProvider } = provider;
  render(
    <LanguageProvider initialLocale="en">
      <Probe />
    </LanguageProvider>,
  );
  // The provider reads the stored settings in a frame after mounting.
  await act(async () => {
    await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
  });
}

const shown = (label: string) => screen.getByLabelText(label).textContent;
const stored = () => JSON.parse(window.localStorage.getItem(STORAGE_KEY) ?? "{}") as Record<string, unknown>;

beforeEach(() => {
  window.localStorage.clear();
  document.documentElement.lang = "en";
});

afterEach(() => {
  vi.doUnmock("@/i18n/messages/pl");
});

describe("switching to a language whose catalog is not here yet", () => {
  it("stores the choice at once and shows the page in it once its catalog arrives", async () => {
    holdPolish();
    await englishPage();
    await mount();

    act(() => screen.getByRole("button", { name: "polski" }).click());

    expect(shown("locale")).toBe("en");
    expect(shown("message")).toBe("Language");
    expect(JSON.parse(shown("switching") ?? "null")).toEqual({ locale: "pl", failed: false });
    expect(stored()["locale"]).toBe("pl");
    expect(document.documentElement.lang).toBe("en");

    await act(async () => release());

    await waitFor(() => expect(shown("locale")).toBe("pl"));
    expect(shown("message")).toBe("Język");
    expect(shown("switching")).toBe("null");
    expect(document.documentElement.lang).toBe("pl");
  });

  it("applies the rest of a change at once", async () => {
    holdPolish();
    await englishPage();
    await mount();

    act(() => screen.getByRole("button", { name: "both" }).click());

    expect(stored()).toMatchObject({ chatTranslationEnabled: true, locale: "pl" });
    expect(shown("locale")).toBe("en");
    await act(async () => release());
    await waitFor(() => expect(shown("locale")).toBe("pl"));
  });

  it("keeps the page's language and says so when the catalog cannot be had", async () => {
    losePolish();
    await englishPage();
    await mount();

    await act(async () => screen.getByRole("button", { name: "polski" }).click());

    await waitFor(() =>
      expect(JSON.parse(shown("switching") ?? "null")).toEqual({ locale: "pl", failed: true }),
    );
    expect(shown("locale")).toBe("en");
    expect(shown("message")).toBe("Language");
    // Stored, so that loading the page again, with the current release's files, opens in it.
    expect(stored()["locale"]).toBe("pl");
  });

  it("lets a later pick win over one still on its way", async () => {
    holdPolish();
    await englishPage();
    await mount();

    act(() => screen.getByRole("button", { name: "polski" }).click());
    act(() => screen.getByRole("button", { name: "english" }).click());
    await act(async () => release());

    expect(shown("locale")).toBe("en");
    expect(shown("switching")).toBe("null");
    expect(stored()["locale"]).toBe("en");
  });
});
