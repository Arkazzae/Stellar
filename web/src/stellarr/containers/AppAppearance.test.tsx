import { render } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import { DEFAULT_THEME, THEME_STORAGE_KEY } from "@/design/themes";
import { ThemeProvider } from "@/state/ThemeProvider";

import { AppAppearance } from "./AppAppearance";

const root = document.documentElement;

afterEach(() => {
  localStorage.clear();
  delete root.dataset["font"];
});

describe("AppAppearance", () => {
  it("sets the face only while the signed-in app is mounted", () => {
    localStorage.setItem(THEME_STORAGE_KEY, JSON.stringify({ ...DEFAULT_THEME, font: "figtree" }));
    // The provider alone, as on the sign-in pages: no face but the brand's.
    const signIn = render(<ThemeProvider>{null}</ThemeProvider>);
    expect(root.dataset["font"]).toBeUndefined();
    signIn.unmount();

    const app = render(
      <ThemeProvider>
        <AppAppearance />
      </ThemeProvider>,
    );
    expect(root.dataset["font"]).toBe("figtree");

    // Signing out leaves the sign-in page in Outfit again.
    app.unmount();
    expect(root.dataset["font"]).toBeUndefined();
  });
});
