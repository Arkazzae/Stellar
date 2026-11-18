import { translateMessage, type Translator } from "@/i18n";

/** A catalog as a translator, for tests that hand one to the code under test. */
export const polish: Translator = (key, values) => translateMessage("pl", key, values);
export const english: Translator = (key, values) => translateMessage("en", key, values);

/**
 * Polish as a query reads the page: the catalog keeps one-letter words on
 * the line of the next one with a non-breaking space, and the queries see
 * every space as a plain one.
 */
export const polishAsRead: Translator = (key, values) => polish(key, values).replace(/\u00a0/g, " ");
