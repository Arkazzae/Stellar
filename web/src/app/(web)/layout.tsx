import type { ReactNode } from "react";

import { requestLocale } from "@/i18n/requestLocale";

import { webDocument, webMetadata, webViewport } from "./document";

import "./globals.css";

export const generateMetadata = webMetadata;

export const viewport = webViewport;

/**
 * The web client's root layout. Cockpit has a root layout of its own beside
 * it (`../cockpit/layout.tsx`), so nothing here reaches the operator panel.
 *
 * A server component, so the static shell stays out of the client bundle and
 * `ThemeScript` can run before the first paint.
 *
 * The language is resolved per request (see `requestLocale`), which is what
 * keeps every route out of static rendering.
 */
export default async function RootLayout({ children }: { children: ReactNode }) {
  return webDocument(await requestLocale(), children);
}
