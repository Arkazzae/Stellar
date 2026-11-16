import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { LANGUAGE_SETTINGS_KEY } from "./defaultLocale";
import {
  INSTANCE_LANGUAGE_GLOBAL,
  INSTANCE_LANGUAGE_PATH,
  instanceLanguageSource,
  LANGUAGE_HINT_KEY,
  LOCALE_SCRIPT,
} from "./localeScript";

type InstanceWindow = Window & { [INSTANCE_LANGUAGE_GLOBAL]?: unknown };

const root = document.documentElement;

/** Runs the instance file, then the locale script, as the document does on load. */
function load(instanceLanguage?: string) {
  if (instanceLanguage !== undefined) (0, eval)(instanceLanguageSource(instanceLanguage));
  (0, eval)(LOCALE_SCRIPT);
}

function choose(locale: unknown) {
  localStorage.setItem(LANGUAGE_SETTINGS_KEY, JSON.stringify({ locale, chatTranslationEnabled: true }));
}

beforeEach(() => {
  localStorage.clear();
  delete (window as InstanceWindow)[INSTANCE_LANGUAGE_GLOBAL];
  root.removeAttribute("lang");
  delete root.dataset.locale;
  document.head.innerHTML = '<meta name="description" content="">';
});

afterEach(() => {
  vi.restoreAllMocks();
  document.head.innerHTML = "";
});

describe("LOCALE_SCRIPT", () => {
  it("is English when the person and the operator chose nothing", () => {
    load();

    expect(root.lang).toBe("en");
    expect(root.dataset.locale).toBe("en");
  });

  it("follows the instance default", () => {
    load("pl");

    expect(root.lang).toBe("pl");
    expect(root.dataset.locale).toBe("pl");
  });

  it("lets the language the person chose on this device win over the instance default", () => {
    choose("en");
    load("pl");
    expect(root.lang).toBe("en");

    choose("pl");
    load("en");
    expect(root.lang).toBe("pl");
  });

  it("ignores a stored choice or an instance default that names an unknown language", () => {
    choose("de");
    load("pl");
    expect(root.lang).toBe("pl");

    load("klingon");
    expect(root.lang).toBe("en");
  });

  it.each(["{not json", "null", "42"])("reads stored %s as no choice", (stored) => {
    localStorage.setItem(LANGUAGE_SETTINGS_KEY, stored);

    load("pl");

    expect(root.lang).toBe("pl");
  });

  it("still decides when storage cannot be read", () => {
    vi.spyOn(localStorage, "getItem").mockImplementation(() => {
      throw new DOMException("denied", "SecurityError");
    });

    load("pl");

    expect(root.lang).toBe("pl");
  });

  it("still decides when the instance file did not load", () => {
    (0, eval)(LOCALE_SCRIPT);

    expect(root.lang).toBe("en");
  });

  it("describes the app in the language it chose", () => {
    load();
    expect(document.querySelector('meta[name="description"]')).toHaveAttribute(
      "content",
      "A small, self-hosted place to talk.",
    );

    choose("pl");
    load();
    expect(document.querySelector('meta[name="description"]')).toHaveAttribute(
      "content",
      "Małe, samodzielnie hostowane miejsce na rozmowy.",
    );
  });

  it("does without a description to rewrite", () => {
    document.head.innerHTML = "";

    load("pl");

    expect(root.lang).toBe("pl");
  });

  it("leaves the language a person with no choice gets as the next load's hint, whatever this one chose", () => {
    choose("en");
    load("pl");
    expect(localStorage.getItem(LANGUAGE_HINT_KEY)).toBe("pl");

    // An operator who names no supported language leaves people in English.
    load("de");
    expect(localStorage.getItem(LANGUAGE_HINT_KEY)).toBe("en");
  });

  it("decides without the hint, which only guesses which catalog to fetch early", () => {
    localStorage.setItem(LANGUAGE_HINT_KEY, "pl");

    load("en");

    expect(root.lang).toBe("en");
    expect(localStorage.getItem(LANGUAGE_HINT_KEY)).toBe("en");
  });
});

describe("the instance file", () => {
  it("is served at one path", () => {
    expect(INSTANCE_LANGUAGE_PATH).toBe("/stellarr-instance.js");
  });

  it("assigns a supported language, and null for anything else", () => {
    expect(instanceLanguageSource("pl")).toBe('window.stellarrInstanceLanguage="pl";\n');
    expect(instanceLanguageSource("en")).toBe('window.stellarrInstanceLanguage="en";\n');
    for (const value of ["", "PL", "de", null, undefined]) {
      expect(instanceLanguageSource(value), String(value)).toBe("window.stellarrInstanceLanguage=null;\n");
    }
  });

  it("sets the global the locale script reads", () => {
    (0, eval)(instanceLanguageSource("pl"));

    expect((window as InstanceWindow)[INSTANCE_LANGUAGE_GLOBAL]).toBe("pl");
  });
});
