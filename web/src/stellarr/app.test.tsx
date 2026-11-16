import { Outlet, Router } from "@stellarr/router";
import { createMemoryHistory } from "@stellarr/router/testing";
import { render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { useLanguage } from "@/state/hooks";

import { StellarrApp } from "./app";
import { routes } from "./routes";

/** Every request is refused as signed out, so nothing leaves the test. */
function signedOut() {
  return vi.fn(async () => new Response(JSON.stringify({ title: "Unauthorized" }), { status: 401 }));
}

function LanguageProbe() {
  return <output aria-label="locale">{useLanguage().locale}</output>;
}

function openAt(href: string) {
  return render(
    <Router routes={routes} history={createMemoryHistory(href)}>
      <StellarrApp>
        <LanguageProbe />
        <Outlet />
      </StellarrApp>
    </Router>,
  );
}

beforeEach(() => {
  vi.stubGlobal("fetch", signedOut());
  localStorage.clear();
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("the web client's document", () => {
  it("renders the page the address names, inside the providers", () => {
    document.documentElement.lang = "en";

    openAt("/nope");

    expect(screen.getByRole("heading", { name: /This page does not exist/ })).toBeInTheDocument();
  });

  it("starts the language provider in the language the pre-paint script chose", () => {
    document.documentElement.lang = "en";
    const { unmount } = openAt("/nope");
    expect(screen.getByLabelText("locale")).toHaveTextContent("en");
    unmount();

    document.documentElement.lang = "pl";
    openAt("/nope");
    expect(screen.getByLabelText("locale")).toHaveTextContent("pl");
    expect(screen.getByRole("heading", { name: /Taka strona nie istnieje/ })).toBeInTheDocument();
  });

  it("keeps the language on <html> that the pre-paint script put there", () => {
    document.documentElement.lang = "en";
    document.documentElement.dataset.locale = "en";

    openAt("/nope");

    expect(document.documentElement.lang).toBe("en");
    expect(document.documentElement.dataset.locale).toBe("en");
  });
});
