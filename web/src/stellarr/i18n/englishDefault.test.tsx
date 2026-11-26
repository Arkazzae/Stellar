import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { LoginScreen } from "@/components/auth/LoginScreen";
import { Avatar } from "@/components/primitives/Avatar";
import { translateMessage } from "@/i18n";
import { LanguageProvider, useLanguage } from "@/state/LanguageProvider";

/**
 * The suite starts every test on a Polish page. These tests are the English
 * one: a page the server rendered with `<html lang="en">`, where components
 * that were handed no translator still speak English.
 */
beforeEach(() => {
  document.documentElement.lang = "en";
});

/** The copy a localized component reads from the provider. */
function LanguageProbe() {
  const { locale, t } = useLanguage();
  return (
    <div>
      <output aria-label="locale">{locale}</output>
      <output aria-label="title">{t("settings.language.title")}</output>
    </div>
  );
}

describe("screens on an English page", () => {
  it("sign-in reads in English without a provider", () => {
    render(<LoginScreen onSubmit={vi.fn()} />);

    expect(screen.getByRole("heading", { name: "Log in" })).toBeInTheDocument();
    expect(screen.getByRole("textbox", { name: "Username or email" })).toBeInTheDocument();
  });

  it("an avatar that opens a profile names it in English", () => {
    render(<Avatar id="u1" displayName="Ada" onOpen={vi.fn()} />);

    expect(screen.getByRole("button", { name: "Profile: Ada" })).toBeInTheDocument();
  });

  it("the provider starts in English when the server chose nothing else", () => {
    render(
      <LanguageProvider>
        <LanguageProbe />
      </LanguageProvider>,
    );

    expect(screen.getByLabelText("locale")).toHaveTextContent("en");
    expect(screen.getByLabelText("title")).toHaveTextContent("Language");
  });
});

describe("the same screens for a person who chose Polish", () => {
  it("keep the Polish interface", () => {
    // The containers hand a screen the translator of the language the person chose.
    render(
      <LanguageProvider initialLocale="pl">
        <LoginScreen onSubmit={vi.fn()} t={(key, values) => translateMessage("pl", key, values)} />
        <Avatar id="u1" displayName="Ada" onOpen={vi.fn()} />
        <LanguageProbe />
      </LanguageProvider>,
    );

    expect(screen.getByRole("heading", { name: "Zaloguj się" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Profil: Ada" })).toBeInTheDocument();
    expect(screen.getByLabelText("locale")).toHaveTextContent("pl");
    expect(screen.getByLabelText("title")).toHaveTextContent("Język");
  });
});

it("shows German sign-in fields and settings when the instance starts in German", () => {
  document.documentElement.lang = "de";
  render(
    <LanguageProvider>
      <LoginScreen onSubmit={vi.fn()} />
      <LanguageProbe />
    </LanguageProvider>,
  );

  expect(screen.getByRole("heading", { name: "Anmelden" })).toBeInTheDocument();
  expect(screen.getByRole("textbox", { name: "Benutzername oder E-Mail-Adresse" })).toBeInTheDocument();
  expect(screen.getByLabelText("locale")).toHaveTextContent("de");
  expect(screen.getByLabelText("title")).toHaveTextContent("Sprache");
});
