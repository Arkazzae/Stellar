import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  INSTANCE_LANGUAGE_GLOBAL,
  InstanceLanguageScript,
  instanceLanguageSource,
  LOCALE_SCRIPT,
} from "./localeScript";

declare global {
  interface Window {
    stellarrInstanceLanguage?: unknown;
  }
}

const SETTINGS_KEY = "stellarr_language_settings";

function setCookie(value: string | null) {
  document.cookie =
    value === null ? "stellarr_locale=; path=/; max-age=0" : `stellarr_locale=${value}; path=/`;
}

/** Runs the script the way the browser does: once, before anything reads the page. */
function runLocaleScript() {
  new Function(LOCALE_SCRIPT)();
  return {
    lang: document.documentElement.lang,
    dataLocale: document.documentElement.dataset.locale,
  };
}

describe("the locale script", () => {
  beforeEach(() => {
    window.localStorage.clear();
    setCookie(null);
    delete window.stellarrInstanceLanguage;
    document.documentElement.lang = "";
    delete document.documentElement.dataset.locale;
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("names the global the instance file sets", () => {
    expect(INSTANCE_LANGUAGE_GLOBAL).toBe("stellarrInstanceLanguage");
  });

  it("falls back to English when nobody chose a language", () => {
    expect(runLocaleScript()).toEqual({ lang: "en", dataLocale: "en" });
  });

  it("follows the instance default on a first visit", () => {
    window.stellarrInstanceLanguage = "pl";
    expect(runLocaleScript()).toEqual({ lang: "pl", dataLocale: "pl" });
  });

  it("lets the remembered choice in the cookie win over the instance default", () => {
    window.stellarrInstanceLanguage = "pl";
    setCookie("en");
    expect(runLocaleScript().lang).toBe("en");

    window.stellarrInstanceLanguage = "en";
    setCookie("pl");
    expect(runLocaleScript().lang).toBe("pl");
  });

  it("puts a choice stored on this device first, as the language provider does", () => {
    window.stellarrInstanceLanguage = "pl";
    setCookie("pl");
    window.localStorage.setItem(SETTINGS_KEY, JSON.stringify({ locale: "en" }));

    expect(runLocaleScript().lang).toBe("en");
  });

  it("ignores stored settings that name no language", () => {
    setCookie("pl");
    window.localStorage.setItem(SETTINGS_KEY, JSON.stringify({ chatTranslationEnabled: true }));
    expect(runLocaleScript().lang).toBe("pl");

    window.localStorage.setItem(SETTINGS_KEY, "not json");
    expect(runLocaleScript().lang).toBe("pl");

    window.localStorage.setItem(SETTINGS_KEY, "null");
    expect(runLocaleScript().lang).toBe("pl");
  });

  it("still decides when storage is blocked", () => {
    vi.spyOn(window.localStorage, "getItem").mockImplementation(() => {
      throw new DOMException("blocked", "SecurityError");
    });
    setCookie("pl");

    expect(runLocaleScript().lang).toBe("pl");
  });

  it("skips values that name no shipped language", () => {
    window.stellarrInstanceLanguage = "pl";
    setCookie("de");
    expect(runLocaleScript().lang).toBe("pl");

    window.stellarrInstanceLanguage = "klingon";
    expect(runLocaleScript().lang).toBe("en");

    window.stellarrInstanceLanguage = null;
    expect(runLocaleScript().lang).toBe("en");
  });

  it("reads its own cookie among others and not one that only ends with its name", () => {
    document.cookie = "xstellarr_locale=pl; path=/";
    document.cookie = "theme=dark; path=/";
    expect(runLocaleScript().lang).toBe("en");

    setCookie("pl");
    expect(runLocaleScript().lang).toBe("pl");
    document.cookie = "xstellarr_locale=; path=/; max-age=0";
    document.cookie = "theme=; path=/; max-age=0";
  });
});

describe("the instance default", () => {
  afterEach(() => {
    vi.unstubAllEnvs();
  });

  it("is published as one assignment of a shipped language, or of null", () => {
    expect(instanceLanguageSource("pl")).toBe('window.stellarrInstanceLanguage="pl";');
    expect(instanceLanguageSource("en")).toBe('window.stellarrInstanceLanguage="en";');
    expect(instanceLanguageSource(undefined)).toBe("window.stellarrInstanceLanguage=null;");
    expect(instanceLanguageSource("")).toBe("window.stellarrInstanceLanguage=null;");
    expect(instanceLanguageSource('"</script><script>alert(1)//')).toBe(
      "window.stellarrInstanceLanguage=null;",
    );
  });

  it("is loaded from the file the container writes, in a build", () => {
    vi.stubEnv("NODE_ENV", "production");
    vi.stubEnv("STELLARR_LANGUAGE", "pl");

    expect(renderToStaticMarkup(<InstanceLanguageScript />)).toBe(
      '<script src="/stellarr-instance.js"></script>',
    );
  });

  it("is written inline from STELLARR_LANGUAGE by the dev server", () => {
    vi.stubEnv("NODE_ENV", "development");
    vi.stubEnv("STELLARR_LANGUAGE", "pl");
    expect(renderToStaticMarkup(<InstanceLanguageScript />)).toBe(
      '<script>window.stellarrInstanceLanguage="pl";</script>',
    );

    vi.stubEnv("STELLARR_LANGUAGE", "fr");
    expect(renderToStaticMarkup(<InstanceLanguageScript />)).toBe(
      "<script>window.stellarrInstanceLanguage=null;</script>",
    );
  });
});
