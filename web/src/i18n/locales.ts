import locales from "../../.locales/meta.json";

export { locales };
export type SupportedLocale = keyof typeof locales;
export const FALLBACK_LOCALE = "en" satisfies SupportedLocale;
export const SUPPORTED_LOCALES = Object.keys(locales) as SupportedLocale[];

export function isSupportedLocale(value: unknown): value is SupportedLocale {
  return typeof value === "string" && Object.hasOwn(locales, value);
}

export function mapLocales<T>(valueFor: (locale: SupportedLocale) => T): Record<SupportedLocale, T> {
  return Object.fromEntries(SUPPORTED_LOCALES.map((code) => [code, valueFor(code)])) as Record<SupportedLocale, T>;
}
