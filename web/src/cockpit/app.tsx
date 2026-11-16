"use client";

import type { ReactNode } from "react";

import { AppGate } from "./components/app-gate";
import { detectLocale, setLocale } from "./i18n";
import { CockpitProvider } from "./state/cockpit-provider";
import { LocaleProvider } from "./state/locale-provider";

// The language is set before anything renders: the one chosen in Settings,
// else English. It lives in this browser's storage, and this module runs
// before the panel's first render.
setLocale(detectLocale());

/**
 * The panel around the page a route renders: the session, the language and
 * the gate that shows the page or the screen that stands in for it.
 */
export function CockpitApp({ children }: { children: ReactNode }) {
  return (
    <CockpitProvider>
      <LocaleProvider>
        <AppGate>{children}</AppGate>
      </LocaleProvider>
    </CockpitProvider>
  );
}
