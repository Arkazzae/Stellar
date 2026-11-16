"use client";

import { useSyncExternalStore, type ReactNode } from "react";

import { AppGate } from "./components/app-gate";
import { detectLocale, setLocale } from "./i18n";
import { CockpitProvider } from "./state/cockpit-provider";
import { LocaleProvider } from "./state/locale-provider";

// The language is set before anything renders: the one chosen in Settings,
// else English. Only the browser knows it, so it is chosen when this module
// first runs there; the server never runs this line.
if (typeof window !== "undefined") setLocale(detectLocale());

const neverChanges = () => () => {};

/**
 * The panel around the page a route renders: the session, the language and
 * the gate that shows the page or the screen that stands in for it.
 *
 * It renders in the browser only. The panel's language lives in this
 * browser's storage and its session comes from the API, so the server has
 * nothing to render in the right words; it sends the document without the
 * panel, and the panel appears once the browser has taken over.
 */
export function CockpitApp({ children }: { children: ReactNode }) {
  const inBrowser = useSyncExternalStore(neverChanges, () => true, () => false);
  if (!inBrowser) return null;

  return (
    <CockpitProvider>
      <LocaleProvider>
        <AppGate>{children}</AppGate>
      </LocaleProvider>
    </CockpitProvider>
  );
}
