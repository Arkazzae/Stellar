import { FALLBACK_LOCALE, locales, mapLocales, SUPPORTED_LOCALES } from "../../i18n/locales";
import { LANGUAGE_SETTINGS_KEY } from "./defaultLocale";

/**
 * Where the instance publishes its default language. One published image
 * serves every instance, so the default cannot be part of the build: the
 * container writes this file from `STELLARR_LANGUAGE` when it starts, and the
 * dev and preview servers answer it from their own `STELLARR_LANGUAGE`. The
 * document loads it with a blocking script before the locale script runs.
 */
export const INSTANCE_LANGUAGE_PATH = "/stellarr-instance.js";

/** The global that file sets: a locale code, or null for an empty or malformed setting. */
export const INSTANCE_LANGUAGE_GLOBAL = "stellarrInstanceLanguage";

/**
 * What the file holds for `language`, the value of `STELLARR_LANGUAGE`: one
 * assignment. The pre-paint script checks this code against the shipped locales.
 *
 *   window.stellarrInstanceLanguage="pl";
 *   window.stellarrInstanceLanguage=null;
 */
export function instanceLanguageSource(language: string | null | undefined): string {
  const value = language && /^[a-z]{2,3}(?:-[A-Za-z0-9]{1,8})*$/u.test(language) ? language : null;
  return `window.${INSTANCE_LANGUAGE_GLOBAL}=${JSON.stringify(value)};\n`;
}

const DESCRIPTIONS = mapLocales((locale) => locales[locale].description);

/**
 * Where this browser keeps the language a person with no choice of their own
 * got on its last load: the instance default, or English. It is a hint and
 * nothing more. The document's first script reads it to start fetching that
 * language's catalog before the instance file has answered (`web/documents.ts`);
 * the language itself is always decided anew below.
 */
export const LANGUAGE_HINT_KEY = "stellarr.language-hint";

/**
 * Runs before paint and puts the page's language on `<html lang>` and
 * `data-locale`, where the language provider and `documentLocale()` read it,
 * and the description in that language on `<meta name="description">`.
 *
 * The order is the provider's: the language the person chose on this device,
 * then the instance default, then English. The static document knows nothing
 * of the person, so this is the only place the choice can be made in time for
 * the first paint. It also leaves the hint for the next load.
 *
 * Like `THEME_SCRIPT`, it repeats `resolveLocale` instead of importing it, and
 * everything it decides with is injected as data from the modules that own it
 * when the document is built.
 */
export const LOCALE_SCRIPT = `(function(){
var SUPPORTED=${JSON.stringify(SUPPORTED_LOCALES)},FALLBACK=${JSON.stringify(FALLBACK_LOCALE)},DESCRIPTIONS=${JSON.stringify(DESCRIPTIONS)};
function pick(value){return SUPPORTED.indexOf(value)<0?null:value}
var stored=null,unchosen=pick(window.${INSTANCE_LANGUAGE_GLOBAL})||FALLBACK;
try{var raw=localStorage.getItem(${JSON.stringify(LANGUAGE_SETTINGS_KEY)});if(raw)stored=JSON.parse(raw).locale}catch(e){}
try{localStorage.setItem(${JSON.stringify(LANGUAGE_HINT_KEY)},unchosen)}catch(e){}
var locale=pick(stored)||unchosen;
var root=document.documentElement;
root.lang=locale;
root.dataset.locale=locale;
var description=document.querySelector('meta[name="description"]');
if(description)description.setAttribute("content",DESCRIPTIONS[locale]);
})()`;
