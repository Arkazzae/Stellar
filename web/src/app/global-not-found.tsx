import { requestLocale } from "@/i18n/requestLocale";

import { webDocument, webMetadata, webViewport } from "./(web)/document";
import NotFound from "./(web)/not-found";

import "./(web)/globals.css";

export const generateMetadata = webMetadata;

export const viewport = webViewport;

/**
 * The page for an address that no route claims, outside /cockpit, where
 * Cockpit's catch-all answers. The web client and Cockpit each have a root
 * layout and nothing sits above them, so there is no layout for a root
 * not-found page to render in: this page is a whole document, the web
 * client's, rendered on the server in the language of the request and sent
 * with a 404.
 */
export default async function GlobalNotFound() {
  return webDocument(await requestLocale(), <NotFound />);
}
