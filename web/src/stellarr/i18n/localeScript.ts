// Relative imports only: the build reads this module from vite.config.ts,
// where the app's `@/` alias does not resolve.
import { isSupportedLocale, SUPPORTED_LOCALES, translateMessage } from ".";
import { FALLBACK_LOCALE, LANGUAGE_SETTINGS_KEY } from "./defaultLocale";

/**
 * Where the instance publishes its default language. One published image
 * serves every instance, so the default cannot be part of the build: the
 * container writes this file from `STELLARR_LANGUAGE` when it starts, and the
 * dev and preview servers answer it from their own `STELLARR_LANGUAGE`. The
 * document loads it with a blocking script before the locale script runs.
 */
export const INSTANCE_LANGUAGE_PATH = "/stellarr-instance.js";

/** The global that file sets: a supported locale, or null when the operator chose none. */
export const INSTANCE_LANGUAGE_GLOBAL = "stellarrInstanceLanguage";

/**
 * What the file holds for `language`, the value of `STELLARR_LANGUAGE`: one
 * assignment, of the language when it is supported and of null otherwise.
 *
 *   window.stellarrInstanceLanguage="pl";
 *   window.stellarrInstanceLanguage=null;
 */
export function instanceLanguageSource(language: string | null | undefined): string {
  const value = isSupportedLocale(language) ? language : null;
  return `window.${INSTANCE_LANGUAGE_GLOBAL}=${JSON.stringify(value)};\n`;
}

const DESCRIPTIONS = Object.fromEntries(
  SUPPORTED_LOCALES.map((locale) => [locale, translateMessage(locale, "app.description")]),
);

/**
 * Runs before paint and puts the page's language on `<html lang>` and
 * `data-locale`, where the language provider and `documentLocale()` read it,
 * and the description in that language on `<meta name="description">`.
 *
 * The order is the provider's: the language the person chose on this device,
 * then the instance default, then English. The static document knows nothing
 * of the person, so this is the only place the choice can be made in time for
 * the first paint.
 *
 * Like `THEME_SCRIPT`, it repeats `resolveLocale` instead of importing it, and
 * everything it decides with is injected as data from the modules that own it
 * when the document is built.
 */
export const LOCALE_SCRIPT = `(function(){
var SUPPORTED=${JSON.stringify(SUPPORTED_LOCALES)},FALLBACK=${JSON.stringify(FALLBACK_LOCALE)},DESCRIPTIONS=${JSON.stringify(DESCRIPTIONS)};
function pick(value){return SUPPORTED.indexOf(value)<0?null:value}
var stored=null;
try{var raw=localStorage.getItem(${JSON.stringify(LANGUAGE_SETTINGS_KEY)});if(raw)stored=JSON.parse(raw).locale}catch(e){}
var locale=pick(stored)||pick(window.${INSTANCE_LANGUAGE_GLOBAL})||FALLBACK;
var root=document.documentElement;
root.lang=locale;
root.dataset.locale=locale;
var description=document.querySelector('meta[name="description"]');
if(description)description.setAttribute("content",DESCRIPTIONS[locale]);
})()`;
