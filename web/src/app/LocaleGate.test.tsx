import { act, render, screen } from "@testing-library/react";
import { hydrateRoot } from "react-dom/client";
import { renderToString } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { LanguageProvider, useLanguage } from "@/state/LanguageProvider";

import { LocaleGate, opensIntoShell } from "./LocaleGate";

const mocks = vi.hoisted(() => ({ pathname: "/" }));

vi.mock("next/navigation", () => ({ usePathname: () => mocks.pathname }));

function Page() {
  const { t } = useLanguage();
  return (
    <main>
      <h1>{t("settings.language.title")}</h1>
      <input aria-label={t("auth.login.identifier")} />
    </main>
  );
}

function App() {
  return (
    <LanguageProvider>
      <LocaleGate>
        <Page />
      </LocaleGate>
    </LanguageProvider>
  );
}

/** The words a render puts on screen or hands to assistive tech. */
function wordsIn(html: string): string[] {
  const template = document.createElement("template");
  template.innerHTML = html;
  const named = [...template.content.querySelectorAll("*")].flatMap((element) =>
    ["aria-label", "title", "alt", "placeholder"].flatMap((name) => element.getAttribute(name) ?? []),
  );
  const text = template.content.textContent ?? "";
  return [...named, ...(text.trim() === "" ? [] : [text])];
}

describe("opensIntoShell", () => {
  it("is true for the home page and for an invite", () => {
    expect(opensIntoShell("/")).toBe(true);
    expect(opensIntoShell("/invite/3f9a0c")).toBe(true);
    expect(opensIntoShell("/invite/_")).toBe(true);
  });

  it("is false for every other address", () => {
    for (const pathname of [
      "/login",
      "/register",
      "/reset-password",
      "/_not-found",
      "/invite",
      "/invite/",
      "/invite/a/b",
      "/index.html",
      "",
      null,
    ]) {
      expect(opensIntoShell(pathname), String(pathname)).toBe(false);
    }
  });
});

describe("LocaleGate", () => {
  beforeEach(() => {
    mocks.pathname = "/";
    window.localStorage.clear();
  });

  afterEach(() => {
    document.documentElement.lang = "pl";
  });

  it("renders the page at once when there is nothing to hydrate", () => {
    render(<App />);
    expect(screen.getByRole("heading", { name: "Język" })).toBeVisible();
  });

  it.each([
    ["the home page", "/"],
    ["an invite", "/invite/_"],
    ["the sign-in page", "/login"],
    ["an unknown address", "/_not-found"],
  ])("leaves every word out of the render made ahead of time for %s", (_case, pathname) => {
    mocks.pathname = pathname;
    const html = renderToString(<App />);

    expect(html).not.toContain("<h1");
    expect(wordsIn(html)).toEqual([]);
  });

  it("draws the shell's skeleton first where the shell is what loads", () => {
    mocks.pathname = "/invite/_";
    expect(renderToString(<App />)).toContain('aria-busy="true"');

    mocks.pathname = "/login";
    expect(renderToString(<App />)).toBe("");
  });

  it("hydrates a page rendered in another language without a mismatch", async () => {
    document.documentElement.lang = "en";
    mocks.pathname = "/invite/_";
    const container = document.createElement("div");
    container.innerHTML = renderToString(<App />);
    document.body.append(container);

    // The browser's locale script has since put the person's language on the page.
    document.documentElement.lang = "pl";
    mocks.pathname = "/invite/3f9a0c";
    const recoverable = vi.fn();
    const consoleError = vi.spyOn(console, "error");
    await act(async () => {
      hydrateRoot(container, <App />, { onRecoverableError: recoverable });
    });

    expect(recoverable).not.toHaveBeenCalled();
    expect(consoleError).not.toHaveBeenCalled();
    expect(container.querySelector("h1")).toHaveTextContent("Język");
    expect(container.querySelector("input")).toHaveAccessibleName("Login lub e-mail");
    consoleError.mockRestore();
    container.remove();
  });
});
