import type { SupportedLocale } from "..";

export interface CatalogUnavailableText {
  title: string;
  message: string;
  action: string;
}

/**
 * What the page says when its own catalog cannot be fetched: the network
 * failed, or a document cached from an older release names a file the server
 * no longer has. Saying it must not need a catalog, so these few lines are the
 * only text the entry carries in every language.
 */
export const CATALOG_UNAVAILABLE: Record<SupportedLocale, CatalogUnavailableText> = {
  en: {
    title: "Stellarr could not load",
    message: "The page's text did not arrive. Check your connection, then load the page again.",
    action: "Load again",
  },
  pl: {
    title: "Nie udało się wczytać Stellarr",
    message: "Teksty strony nie dotarły. Sprawdź połączenie i wczytaj stronę ponownie.",
    action: "Wczytaj ponownie",
  },
};
