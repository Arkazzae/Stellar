import { isSupportedLocale, translateMessage, type SupportedLocale, type Translator } from ".";

/** What a person sees when neither they nor the instance's operator chose a language. */
export const FALLBACK_LOCALE: SupportedLocale = "en";

/**
 * Where this browser keeps the language settings: the interface language
 * the person picked, if they did, and the chat translation preferences.
 */
export const LANGUAGE_SETTINGS_KEY = "stellarr_language_settings";

/** The first candidate that names a shipped locale, or English when none does. */
export function resolveLocale(
  ...candidates: ReadonlyArray<string | null | undefined>
): SupportedLocale {
  return candidates.find(isSupportedLocale) ?? FALLBACK_LOCALE;
}

/**
 * The language the page is in right now.
 *
 * `<html lang>` is where the document's pre-paint script puts the language it
 * chose and where the language provider keeps the person's current one. Only
 * entry points that can be called with nothing to ask read it, at call time: a
 * component rendered without a provider, or a formatter or capture function
 * called without a locale or translator. Code that runs inside the app is
 * handed the provider's translator or returns catalog keys instead, so it
 * follows the person's settings and not this attribute. With no document there
 * is nothing to read.
 */
export function documentLocale(): SupportedLocale {
  if (typeof document === "undefined") return FALLBACK_LOCALE;
  return resolveLocale(document.documentElement.lang);
}

/** A translator for the page's language, for callers that were handed none. */
export const translateForDocument: Translator = (key, values) =>
  translateMessage(documentLocale(), key, values);
