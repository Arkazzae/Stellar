import { FALLBACK_LOCALE, isSupportedLocale, type SupportedLocale } from "../../i18n/locales";
import catalogs from "../../../.locales/cockpit.json";

export { locales, SUPPORTED_LOCALES as LOCALES } from "../../i18n/locales";
export type Locale = SupportedLocale;
export type MessageKey = keyof typeof catalogs.en;
export type MessageValues = Record<string, string | number>;

/** Where this browser keeps the panel's language. */
const LOCALE_KEY = "stellarr.cockpit.locale";

/**
 * The panel's language: the one chosen in its settings, else English. The
 * panel does not follow the browser or the web client; the operator picks.
 */
export function resolveLocale(stored: string | null): Locale {
  return isSupportedLocale(stored) ? stored : FALLBACK_LOCALE;
}

export function detectLocale(): Locale {
  try {
    return resolveLocale(localStorage.getItem(LOCALE_KEY));
  } catch {
    // Storage can be off (a private window, blocked site data): English it is.
    return FALLBACK_LOCALE;
  }
}

/** Remembers the panel's language in this browser. */
export function saveLocale(locale: Locale): void {
  try {
    localStorage.setItem(LOCALE_KEY, locale);
  } catch {
    // Without storage the choice holds until the page is closed.
  }
}

let active: Locale = FALLBACK_LOCALE;

/** Selects the language of every later lookup and format, and of the document. */
export function setLocale(locale: Locale): void {
  active = locale;
  document.documentElement.lang = locale;
}

export function getLocale(): Locale {
  return active;
}

/** Looks a message up in the active language and fills its `{name}` placeholders. */
export function t(key: MessageKey, values: MessageValues = {}): string {
  return catalogs[active][key].replace(/\{([^}]+)\}/g, (token, name: string) => {
    const value = values[name];
    return value === undefined ? token : String(value);
  });
}
