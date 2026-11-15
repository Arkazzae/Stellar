import { isSupportedLocale, SUPPORTED_LOCALES } from ".";
import { FALLBACK_LOCALE, LANGUAGE_SETTINGS_KEY, LOCALE_COOKIE } from "./defaultLocale";

/**
 * Where the container publishes the instance default. `infra/entrypoint.sh`
 * writes the file from `STELLARR_LANGUAGE` at every start and Nginx serves it
 * with the HTML's cache policy, so a changed setting reaches the next page
 * load. One published image serves every instance, so the default cannot be
 * part of the build.
 */
export const INSTANCE_LANGUAGE_PATH = "/stellarr-instance.js";

/** The global that file sets: a supported locale, or null when the operator chose none. */
export const INSTANCE_LANGUAGE_GLOBAL = "stellarrInstanceLanguage";

/**
 * What the file holds. `infra/entrypoint.sh` writes the same assignment, and
 * only for a supported language.
 */
export function instanceLanguageSource(language: string | null | undefined): string {
  const value = isSupportedLocale(language) ? language : null;
  return `window.${INSTANCE_LANGUAGE_GLOBAL}=${JSON.stringify(value)};`;
}

/**
 * Loads the instance default before the locale script reads it. It blocks the
 * parser on purpose: the language has to be known before the first paint.
 *
 * `next dev` has no Nginx and no entrypoint, so there the dev server writes
 * the same assignment inline from its own `STELLARR_LANGUAGE`.
 */
export function InstanceLanguageScript() {
  if (process.env.NODE_ENV === "development") {
    return (
      <script
        dangerouslySetInnerHTML={{ __html: instanceLanguageSource(process.env.STELLARR_LANGUAGE) }}
      />
    );
  }
  // eslint-disable-next-line @next/next/no-sync-scripts -- it must run before the first paint
  return <script src={INSTANCE_LANGUAGE_PATH} />;
}

/**
 * Runs before paint and puts the page's language on `<html lang>` and
 * `data-locale`, where the language provider and `documentLocale()` read it.
 *
 * The order is the provider's: a language the person chose on this device,
 * then the cookie that remembers it (Cockpit reads it too), then the
 * instance default, then English. Nothing about the person is known to the
 * static HTML, so this is the only place the choice can be made in time.
 *
 * Like `ThemeScript`, it repeats `resolveLocale` instead of importing it, and
 * everything it decides with is injected as data from the modules that own
 * it.
 */
export const LOCALE_SCRIPT = `(function(){
var SUPPORTED=${JSON.stringify(SUPPORTED_LOCALES)},FALLBACK=${JSON.stringify(FALLBACK_LOCALE)};
function pick(value){return SUPPORTED.indexOf(value)<0?null:value}
var stored=null,cookie=null;
try{var raw=localStorage.getItem(${JSON.stringify(LANGUAGE_SETTINGS_KEY)});if(raw)stored=JSON.parse(raw).locale}catch(e){}
var parts=document.cookie.split(";");
for(var i=0;i<parts.length;i++){
var at=parts[i].indexOf("=");
if(at>0&&parts[i].slice(0,at).trim()===${JSON.stringify(LOCALE_COOKIE)}){cookie=parts[i].slice(at+1).trim();break}
}
var locale=pick(stored)||pick(cookie)||pick(window.${INSTANCE_LANGUAGE_GLOBAL})||FALLBACK;
var root=document.documentElement;
root.lang=locale;
root.dataset.locale=locale;
})()`;

export function LocaleScript() {
  return <script dangerouslySetInnerHTML={{ __html: LOCALE_SCRIPT }} />;
}
