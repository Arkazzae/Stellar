import { Children, isValidElement, type ReactElement, type ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, describe, expect, it, vi } from "vitest";

import { ThemeScript } from "@/design/themeScript";
import { InstanceLanguageScript, LocaleScript } from "@/i18n/localeScript";
import { LanguageProvider } from "@/state/LanguageProvider";

import RootLayout, { metadata } from "./layout";
import { LocaleGate } from "./LocaleGate";

type Element = ReactElement<Record<string, unknown>>;

/** The first element of `type` anywhere under `node`. */
function find(node: ReactNode, type: unknown): Element | null {
  if (Array.isArray(node)) {
    for (const child of node) {
      const found = find(child, type);
      if (found !== null) return found;
    }
    return null;
  }
  if (!isValidElement<Record<string, unknown>>(node)) return null;
  return node.type === type ? node : find(node.props["children"] as ReactNode, type);
}

const page = <p>page</p>;

function layout(): Element {
  return RootLayout({ children: page }) as Element;
}

describe("the root layout", () => {
  afterEach(() => {
    vi.unstubAllEnvs();
    document.cookie = "stellarr_locale=; path=/; max-age=0";
  });

  it("leaves the language off <html> for the locale script to write", () => {
    const html = layout();

    expect(html.type).toBe("html");
    expect(html.props).not.toHaveProperty("lang");
    expect(html.props).not.toHaveProperty("data-locale");
    expect(html.props["suppressHydrationWarning"]).toBe(true);
  });

  it("loads the instance default, then picks the language, then the theme, all in the head", () => {
    const head = find(layout(), "head");
    const scripts = Children.toArray(head?.props["children"] as ReactNode).map(
      (child) => (child as Element).type,
    );

    expect(scripts).toEqual([InstanceLanguageScript, LocaleScript, ThemeScript]);
  });

  it("starts the language provider in the page's language and holds the page for the browser", () => {
    const html = layout();

    expect(find(html, LanguageProvider)?.props["initialLocale"]).toBeUndefined();
    expect(find(html, LocaleGate)?.props["children"]).toBe(page);
  });

  it("renders the same head whatever the instance or the person chose", () => {
    const head = () => renderToStaticMarkup(find(layout(), "head"));

    vi.stubEnv("STELLARR_LANGUAGE", "en");
    const english = head();
    vi.stubEnv("STELLARR_LANGUAGE", "pl");
    document.cookie = "stellarr_locale=pl; path=/";

    expect(head()).toBe(english);
    expect(english).toContain('<script src="/stellarr-instance.js"></script>');
  });

  it("keeps the pages out of indexes and describes them in no language", () => {
    expect(metadata).toEqual({
      title: "Stellarr",
      applicationName: "Stellarr",
      robots: { index: false, follow: false },
    });
  });
});
