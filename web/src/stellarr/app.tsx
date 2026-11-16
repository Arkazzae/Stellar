import type { ReactNode } from "react";

import { LanguageProvider } from "@/state/LanguageProvider";
import { PortsProvider } from "@/state/PortsProvider";
import { SessionProvider } from "@/state/SessionProvider";
import { ThemeProvider } from "@/state/ThemeProvider";

/**
 * The providers around every page of the web client, /login included,
 * because signing in needs the ports and the session. The app's own providers
 * start further down, at `AppContainer`.
 *
 * The language provider starts in the language the document's pre-paint
 * script put on `<html lang>` (`i18n/localeScript.ts`), so the first render
 * speaks the language of the first paint.
 */
export function StellarrApp({ children }: { children: ReactNode }) {
  return (
    <LanguageProvider>
      <ThemeProvider>
        <PortsProvider>
          <SessionProvider>{children}</SessionProvider>
        </PortsProvider>
      </ThemeProvider>
    </LanguageProvider>
  );
}
