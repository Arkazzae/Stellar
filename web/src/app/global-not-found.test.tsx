import { isValidElement, type ReactElement, type ReactNode } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const cookieJar = new Map<string, string>();

vi.mock("next/headers", () => ({
  cookies: async () => ({
    get: (name: string) => {
      const value = cookieJar.get(name);
      return value === undefined ? undefined : { name, value };
    },
  }),
}));

import { LanguageProvider } from "@/state/LanguageProvider";

import NotFound from "./(web)/not-found";
import GlobalNotFound from "./global-not-found";

/** The first element of `type` anywhere under `node`. */
function find(node: ReactNode, type: unknown): ReactElement<Record<string, unknown>> | null {
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

describe("the page for an address no route claims", () => {
  beforeEach(() => {
    cookieJar.clear();
    vi.stubEnv("STELLARR_LANGUAGE", undefined);
  });

  afterEach(() => {
    vi.unstubAllEnvs();
  });

  it("is the web client's not-found page in the web client's document", async () => {
    const page = (await GlobalNotFound()) as ReactElement<Record<string, unknown>>;

    expect(page.type).toBe("html");
    expect(find(page, NotFound)).not.toBeNull();
  });

  it("speaks the language of the request", async () => {
    vi.stubEnv("STELLARR_LANGUAGE", "pl");
    const instance = (await GlobalNotFound()) as ReactElement<Record<string, unknown>>;
    expect(instance.props["lang"]).toBe("pl");
    expect(find(instance, LanguageProvider)?.props["initialLocale"]).toBe("pl");

    cookieJar.set("stellarr_locale", "en");
    const chosen = (await GlobalNotFound()) as ReactElement<Record<string, unknown>>;
    expect(chosen.props["lang"]).toBe("en");
  });
});
