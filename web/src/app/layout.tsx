import type { Metadata, Viewport } from "next";
import type { ReactNode } from "react";

import { LegacyPwaCleanup } from "@/components/platform/LegacyPwaCleanup";
import { ThemeScript } from "@/design/themeScript";
import { InstanceLanguageScript, LocaleScript } from "@/i18n/localeScript";
import { LanguageProvider } from "@/state/LanguageProvider";
import { PortsProvider } from "@/state/PortsProvider";
import { SessionProvider } from "@/state/SessionProvider";
import { ThemeProvider } from "@/state/ThemeProvider";

import { LocaleGate } from "./LocaleGate";

import "./globals.css";

// No description: it would be words in a language fixed at build time. The
// pages are kept out of indexes, so only a link preview would have shown it.
export const metadata: Metadata = {
  title: "Stellarr",
  applicationName: "Stellarr",
  robots: { index: false, follow: false },
};

export const viewport: Viewport = {
  themeColor: "#111111",
  width: "device-width",
  initialScale: 1,
  viewportFit: "cover",
  // Zooming to 200% is a WCAG requirement, so the scale is never locked.
  maximumScale: 5,
};

/**
 * A server component, so the static shell stays out of the client bundle and
 * the head scripts run before the first paint.
 *
 * Every page is rendered ahead of time, the same for every person and every
 * instance. The language is chosen in the browser: the instance default
 * arrives as a small script, the locale script picks the person's language
 * and puts it on `<html lang>`, and `LocaleGate` renders the page once React
 * has taken over, in that language.
 *
 * Ports and the session wrap every route, /login included, because signing in
 * needs both. The app's own providers start further down, at `AppContainer`.
 */
export default function RootLayout({ children }: { children: ReactNode }) {
  return (
    // The locale and theme scripts write lang, data-locale, data-theme, -scheme
    // and -contrast onto <html> before React hydrates, so the client can never
    // match the server's bare element. Those attributes belong to the scripts;
    // the suppression covers this one element and none of its children.
    <html suppressHydrationWarning>
      <head>
        <InstanceLanguageScript />
        <LocaleScript />
        <ThemeScript />
      </head>
      <body>
        <LegacyPwaCleanup />
        <LanguageProvider>
          <ThemeProvider>
            <PortsProvider>
              <SessionProvider>
                <LocaleGate>{children}</LocaleGate>
              </SessionProvider>
            </PortsProvider>
          </ThemeProvider>
        </LanguageProvider>
      </body>
    </html>
  );
}
