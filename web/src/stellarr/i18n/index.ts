import { SUPPORTED_LOCALES, type SupportedLocale } from "../../i18n/locales";
import type en from "../../../.locales/web/en.json";

export { isSupportedLocale, locales, SUPPORTED_LOCALES, type SupportedLocale } from "../../i18n/locales";
export type InterfaceLocale = SupportedLocale;
// Target languages of the bundled offline translation models.
export const TRANSLATION_TARGETS = ["en", "pl"] as const;
export type TranslationTargetLanguage = (typeof TRANSLATION_TARGETS)[number];

export function isTranslationTarget(value: unknown): value is TranslationTargetLanguage {
  return TRANSLATION_TARGETS.some((target) => target === value);
}

export interface LanguageSettings {
  locale: InterfaceLocale;
  chatTranslationEnabled: boolean;
  chatAutoTranslateEnabled: boolean;
  chatTranslationTargetLanguage: TranslationTargetLanguage;
}

/** Everything but the locale, which comes from the request and not from a default. */
export const TRANSLATION_DEFAULTS = {
  chatTranslationEnabled: false,
  chatAutoTranslateEnabled: false,
  chatTranslationTargetLanguage: "en",
} as const satisfies Omit<LanguageSettings, "locale">;

export type MessageKey = keyof typeof en;
export type MessageValues = Record<string, string | number>;
export type Translator = (key: MessageKey, values?: MessageValues) => string;
export type MessageCatalog = Record<MessageKey, string>;

/**
 * Each catalog is a chunk of its own, so a page fetches only its language.
 * The document starts fetching that one next to the entry (`web/documents.ts`),
 * and `main.tsx` renders once it is here; another one is fetched when the
 * person picks its language.
 */
async function fetchCatalog(locale: SupportedLocale): Promise<MessageCatalog> {
  return (await import(`../../../.locales/web/${locale}.json`)).default;
}

const catalogs: Partial<Record<SupportedLocale, MessageCatalog>> = {};
const pending: Partial<Record<SupportedLocale, Promise<void>>> = {};

/** Whether `locale` can be translated into right now, without waiting. */
export function catalogLoaded(locale: SupportedLocale): boolean {
  return catalogs[locale] !== undefined;
}

/**
 * Fetches a language's catalog, once: callers that overlap share the request,
 * and a failed one is asked again by the next caller rather than remembered.
 */
export function loadCatalog(locale: SupportedLocale): Promise<void> {
  if (catalogs[locale] !== undefined) return Promise.resolve();
  const inFlight = pending[locale];
  if (inFlight !== undefined) return inFlight;
  const request = fetchCatalog(locale).then(
    (catalog) => {
      catalogs[locale] = catalog;
      delete pending[locale];
    },
    (cause: unknown) => {
      delete pending[locale];
      throw cause;
    },
  );
  pending[locale] = request;
  return request;
}

/**
 * The catalog to translate into. The app renders only once the page's own is
 * here and switches language only once the new one is, so `locale` is always
 * loaded in practice; were it not, the text comes in the language that is
 * loaded rather than as raw keys. With no catalog at all the call is a bug in
 * the order of startup, and it says so.
 */
function catalogFor(locale: SupportedLocale): MessageCatalog {
  const catalog = catalogs[locale] ?? SUPPORTED_LOCALES.map((other) => catalogs[other]).find(Boolean);
  if (catalog === undefined) {
    throw new Error(`No message catalog is loaded yet; "${locale}" was asked for before loadCatalog.`);
  }
  return catalog;
}

/**
 * Looks a message up and fills its `{name}` placeholders. A placeholder with
 * no matching value stays as written, so a missing argument is visible in the
 * UI instead of rendering as "undefined".
 */
export function translateMessage(
  locale: SupportedLocale,
  key: MessageKey,
  values: MessageValues = {},
): string {
  return catalogFor(locale)[key].replace(/\{([^}]+)\}/g, (placeholder, name: string) => {
    const value = values[name];
    return value === undefined ? placeholder : String(value);
  });
}
