import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";

import {
  TRANSLATION_DEFAULTS,
  isSupportedLocale,
  translateMessage,
  type InterfaceLocale,
  type LanguageSettings,
  type MessageKey,
  type MessageValues,
} from "@/i18n";
import { documentLocale, LANGUAGE_SETTINGS_KEY } from "@/i18n/defaultLocale";

export type { InterfaceLocale, LanguageSettings, TranslationTargetLanguage } from "@/i18n";

export interface LanguageValue extends LanguageSettings {
  update: (patch: Partial<LanguageSettings>) => void;
  /**
   * The interface language the person picked, now or on an earlier visit, or
   * undefined while they have not: what shows then is the instance default,
   * which nothing should store on their behalf.
   */
  chosenLocale: () => InterfaceLocale | undefined;
  /** Short component copy in both languages, without a catalog entry. */
  text: (polish: string, english: string) => string;
  /** Typed catalog lookup for copy shared across surfaces. */
  t: (key: MessageKey, values?: MessageValues) => string;
}


const DEFAULTS = TRANSLATION_DEFAULTS;

function readStored(fallbackLocale: InterfaceLocale): LanguageSettings {
  try {
    const raw = window.localStorage.getItem(LANGUAGE_SETTINGS_KEY);
    if (raw === null) return { ...DEFAULTS, locale: fallbackLocale };
    const stored = JSON.parse(raw) as Partial<LanguageSettings>;
    const translationEnabled = stored.chatTranslationEnabled === true;
    return {
      locale: isSupportedLocale(stored.locale) ? stored.locale : fallbackLocale,
      chatTranslationEnabled: translationEnabled,
      chatAutoTranslateEnabled: translationEnabled && stored.chatAutoTranslateEnabled === true,
      chatTranslationTargetLanguage: isSupportedLocale(stored.chatTranslationTargetLanguage)
        ? stored.chatTranslationTargetLanguage
        : DEFAULTS.chatTranslationTargetLanguage,
    };
  } catch {
    return { ...DEFAULTS, locale: fallbackLocale };
  }
}

/**
 * Stores the device's language settings. The locale goes in only when the
 * person picked one, now or before: what the provider shows when nobody chose
 * is the instance default, and storing it would pin the person to that default
 * after the operator changes it. The document's pre-paint script reads the
 * same entry (`i18n/localeScript.ts`), so the next load starts in the choice.
 */
function persist(settings: LanguageSettings, localeChosen: boolean): void {
  try {
    const { locale, ...device } = settings;
    const kept = localeChosen ? locale : readStoredLocale();
    window.localStorage.setItem(
      LANGUAGE_SETTINGS_KEY,
      JSON.stringify(kept === undefined ? device : { ...device, locale: kept }),
    );
  } catch {
    // A private or locked-down browser may refuse storage. The choice still
    // holds for this tab, and translation keeps working.
  }
}

/** The language the person chose on an earlier visit, if they ever did. */
function readStoredLocale(): InterfaceLocale | undefined {
  try {
    const raw = window.localStorage.getItem(LANGUAGE_SETTINGS_KEY);
    if (raw === null) return undefined;
    const locale = (JSON.parse(raw) as { locale?: unknown } | null)?.locale;
    return isSupportedLocale(locale) ? locale : undefined;
  } catch {
    return undefined;
  }
}

function applyPatch(current: LanguageSettings, patch: Partial<LanguageSettings>): LanguageSettings {
  const next = { ...current, ...patch };
  const pairChanged =
    patch.chatTranslationTargetLanguage !== undefined &&
    patch.chatTranslationTargetLanguage !== current.chatTranslationTargetLanguage;
  // A new language pair needs its own model, so auto-translation stays off
  // until the user sees that model is ready and turns it on again.
  if (!next.chatTranslationEnabled || pairChanged) next.chatAutoTranslateEnabled = false;
  return next;
}

function standaloneValue(locale: InterfaceLocale): LanguageValue {
  return {
    ...DEFAULTS,
    locale,
    update: () => undefined,
    chosenLocale: () => undefined,
    text: (polish, english) => (locale === "en" ? english : polish),
    t: (key, values) => translateMessage(locale, key, values),
  };
}

// What a component sees when no provider is mounted, as in most component
// tests: the page's language, and changes go nowhere. One value per locale, so
// the identity of `t` stays stable between renders.
const STANDALONE: Record<InterfaceLocale, LanguageValue> = {
  pl: standaloneValue("pl"),
  en: standaloneValue("en"),
};

const LanguageContext = createContext<LanguageValue | null>(null);

export function LanguageProvider({
  children,
  initialLocale,
}: {
  children: ReactNode;
  /**
   * The language to start in. Without one, the page's own: what the
   * pre-paint script put on `<html lang>`.
   */
  initialLocale?: InterfaceLocale;
}) {
  // Read once: the provider itself rewrites `<html lang>` when the locale
  // changes, and that must not look like a new starting point.
  const [pageLocale] = useState(documentLocale);
  const startLocale = initialLocale ?? pageLocale;
  // The first render speaks the page's language, which the pre-paint script
  // already chose from the same storage. Translation preferences are kept in
  // this browser (the account's copy arrives later, through
  // ClientSettingsSync), so they are read after the first frame.
  const [settings, setSettings] = useState<LanguageSettings>({
    ...DEFAULTS,
    locale: startLocale,
  });
  // Lets `update` see changes made earlier in the same tick, before React
  // has re-rendered with them.
  const latest = useRef(settings);

  const commit = useCallback((next: LanguageSettings) => {
    latest.current = next;
    setSettings(next);
  }, []);

  useEffect(() => {
    const frame = requestAnimationFrame(() => {
      commit(readStored(startLocale));
    });
    return () => cancelAnimationFrame(frame);
  }, [startLocale, commit]);

  useEffect(() => {
    document.documentElement.lang = settings.locale;
    document.documentElement.dataset.locale = settings.locale;
  }, [settings.locale]);

  const update = useCallback(
    (patch: Partial<LanguageSettings>) => {
      const current = latest.current;
      const next = applyPatch(current, patch);
      commit(next);
      persist(next, patch.locale !== undefined);
    },
    [commit],
  );

  const { locale } = settings;
  const text = useCallback(
    (polish: string, english: string) => (locale === "en" ? english : polish),
    [locale],
  );
  const t = useCallback(
    (key: MessageKey, values?: MessageValues) => translateMessage(locale, key, values),
    [locale],
  );

  const value = useMemo<LanguageValue>(
    () => ({ ...settings, update, chosenLocale: readStoredLocale, text, t }),
    [settings, update, text, t],
  );

  return <LanguageContext.Provider value={value}>{children}</LanguageContext.Provider>;
}

export function useLanguage(): LanguageValue {
  return useContext(LanguageContext) ?? STANDALONE[documentLocale()];
}
