import type { Metadata, Viewport } from "next";
import type { ReactNode } from "react";
import { CockpitApp } from "@/cockpit/app";

import "@/cockpit/index.css";

// Rendered on each request, as the web client's pages are. A prerendered
// page would go out with s-maxage=31536000, and a proxy in front of the
// instance could then serve a release's HTML long after the next one.
export const dynamic = "force-dynamic";

export const metadata: Metadata = {
  title: "Stellarr Cockpit",
  robots: { index: false, follow: false },
};

export const viewport: Viewport = {
  width: "device-width",
  initialScale: 1,
  themeColor: [
    { media: "(prefers-color-scheme: light)", color: "#ffffff" },
    { media: "(prefers-color-scheme: dark)", color: "#1b1b1f" },
  ],
};

/**
 * The root layout of Cockpit, the operator panel, whose code is in
 * `src/cockpit/`. It sits beside the web client's root layout in `(web)/` and
 * shares nothing with it: its own document, stylesheet, language and session.
 * Following a link from one to the other is a full page load, as it is between
 * any two root layouts.
 *
 * The panel renders in the browser only (see `CockpitApp`), so this document
 * starts in the panel's default language and the panel sets `lang` once it
 * knows its own. The suppression covers that one attribute on this one
 * element.
 */
export default function CockpitLayout({ children }: { children: ReactNode }) {
  return (
    <html lang="en" suppressHydrationWarning>
      <body>
        <CockpitApp>{children}</CockpitApp>
      </body>
    </html>
  );
}
