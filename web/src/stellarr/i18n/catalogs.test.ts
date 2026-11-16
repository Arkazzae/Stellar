import { afterEach, describe, expect, it, vi } from "vitest";

/** A registry with no catalog in it, as a page has before `main.tsx` loads one. */
async function freshRegistry() {
  vi.resetModules();
  return await import(".");
}

afterEach(() => {
  vi.doUnmock("./messages/en");
  vi.doUnmock("./messages/pl");
});

describe("the message catalogs", () => {
  it("are loaded one language at a time", async () => {
    const i18n = await freshRegistry();
    expect(i18n.catalogLoaded("en")).toBe(false);
    expect(i18n.catalogLoaded("pl")).toBe(false);

    await i18n.loadCatalog("en");

    expect(i18n.catalogLoaded("en")).toBe(true);
    expect(i18n.catalogLoaded("pl")).toBe(false);
    expect(i18n.translateMessage("en", "settings.language.title")).toBe("Language");
  });

  it("share one request between callers that overlap", async () => {
    const i18n = await freshRegistry();

    const first = i18n.loadCatalog("pl");
    const second = i18n.loadCatalog("pl");

    expect(second).toBe(first);
    await first;
    expect(i18n.translateMessage("pl", "settings.language.title")).toBe("Język");
  });

  it("ask again after a failed fetch instead of remembering it", async () => {
    vi.doMock("./messages/en", () => {
      throw new Error("offline");
    });
    const i18n = await freshRegistry();

    await expect(i18n.loadCatalog("en")).rejects.toThrow();
    expect(i18n.catalogLoaded("en")).toBe(false);

    vi.doUnmock("./messages/en");
    await i18n.loadCatalog("en");
    expect(i18n.catalogLoaded("en")).toBe(true);
  });

  it("give the page's language when another one is asked for before it has come", async () => {
    const i18n = await freshRegistry();
    await i18n.loadCatalog("pl");

    expect(i18n.translateMessage("en", "settings.language.title")).toBe("Język");
  });

  it("refuse a lookup before any catalog is loaded, which only a startup bug can cause", async () => {
    const i18n = await freshRegistry();

    expect(() => i18n.translateMessage("en", "settings.language.title")).toThrow(/No message catalog/);
  });
});
