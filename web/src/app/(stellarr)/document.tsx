import type { Metadata, Viewport } from "next";
import type { ReactElement, ReactNode } from "react";

import { ThemeScript } from "@/design/themeScript";
import { translateMessage, type SupportedLocale } from "@/i18n";
import { requestLocale } from "@/i18n/requestLocale";
import { LanguageProvider } from "@/state/LanguageProvider";
import { PortsProvider } from "@/state/PortsProvider";
import { SessionProvider } from "@/state/SessionProvider";
import { ThemeProvider } from "@/state/ThemeProvider";

/** The web client's metadata, in the language of the request. */
export async function webMetadata(): Promise<Metadata> {
  return {
    title: "Stellarr",
    description: translateMessage(await requestLocale(), "app.description"),
    applicationName: "Stellarr",
    robots: { index: false, follow: false },
  };
}

export const webViewport: Viewport = {
  themeColor: "#111111",
  width: "device-width",
  initialScale: 1,
  viewportFit: "cover",
  // Zooming to 200% is a WCAG requirement, so the scale is never locked.
  maximumScale: 5,
};

/**
 * The web client's document around `children`, in `locale`: the root layout
 * renders every page in it, and `global-not-found.tsx` the page for an
 * address no route claims.
 *
 * Ports and the session wrap every route, /login included, because signing in
 * needs both. The app's own providers start further down, at `AppContainer`.
 *
 * The language goes to the client through `<html lang>` and the provider, so
 * both agree with the server's render.
 */
export function webDocument(locale: SupportedLocale, children: ReactNode): ReactElement {
  return (
    // ThemeScript writes data-theme, -scheme and -contrast onto <html> before
    // React hydrates, so the client can never match the server's bare element.
    // Those attributes belong to the script; the suppression covers this one
    // element and none of its children.
    <html lang={locale} data-locale={locale} suppressHydrationWarning>
      <head>
        <ThemeScript />
      </head>
      <body>
        <LanguageProvider initialLocale={locale}>
          <ThemeProvider>
            <PortsProvider>
              <SessionProvider>{children}</SessionProvider>
            </PortsProvider>
          </ThemeProvider>
        </LanguageProvider>
      </body>
    </html>
  );
}
