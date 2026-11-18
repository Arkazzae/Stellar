import type { ReactNode } from "react";

import { InstanceSetupProvider } from "@/state/InstanceSetupProvider";
import { LanguageProvider } from "@/state/LanguageProvider";
import { PortsProvider } from "@/state/PortsProvider";
import { SessionProvider } from "@/state/SessionProvider";
import { ThemeProvider } from "@/state/ThemeProvider";

/**
 * The providers around every page of the web client, /login included,
 * because signing in needs the ports and the session, and every page leads
 * to /setup while the instance has no owner. The app's own providers start
 * further down, at `AppContainer`.
 *
 * The language provider starts in the language the document's pre-paint
 * script put on `<html lang>` (`i18n/localeScript.ts`), whose catalog
 * `main.tsx` has loaded, so the first render speaks the language of the first
 * paint.
 */
export function StellarrApp({ children }: { children: ReactNode }) {
  return (
    <LanguageProvider>
      <ThemeProvider>
        <PortsProvider>
          <SessionProvider>
            <InstanceSetupProvider>{children}</InstanceSetupProvider>
          </SessionProvider>
        </PortsProvider>
      </ThemeProvider>
    </LanguageProvider>
  );
}
