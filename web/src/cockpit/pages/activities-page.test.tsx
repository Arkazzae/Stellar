import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type * as CockpitApiModule from "../lib/api";
import { CockpitApiError, cockpitRequest } from "../lib/api";
import { selectPolish } from "../test/locale";
import { ActivitiesPage } from "./activities-page";

selectPolish();
vi.mock("../lib/api", async (importOriginal) => ({
  ...(await importOriginal<typeof CockpitApiModule>()),
  cockpitRequest: vi.fn(),
}));

const api = vi.mocked(cockpitRequest);
const base = "/api/cockpit/activities";
const url = "https://github.com/example/games";

/** What the page sent to one endpoint, in order. */
const sent = (path: string) => api.mock.calls.filter(([called]) => called === base + path).map(([, options]) => options?.body);
const row = (name: string) => screen.getByRole("row", { name: new RegExp(name) });

interface Version {
  id: string;
  name: string;
  version: string;
  permissions?: string[];
  connects?: { host: string; why: string }[];
}

/** A published version of an activity in the example repository. */
function release({ id, name, version, permissions = [], connects = [] }: Version) {
  return {
    artwork: { icon: `data:image/png;base64,${id}`, cover: null as string | null },
    source: url,
    release: `${url}/releases/tag/${id}/v${version}`,
    digest: version.replace(/\D/g, "").padEnd(64, "d"),
    revision: "e".repeat(40),
    bytes: 8_300_000,
    manifest: { id, name, version, description: `Play ${name}`, permissions, connects },
  };
}

// A block body: Vitest would run a returned mock as the test's teardown.
beforeEach(() => {
  api.mockReset();
});

describe("ActivitiesPage", () => {
  it("lists what is installed with its version, source, access and state", async () => {
    const music = release({
      id: "music",
      name: "Music",
      version: "1.0.0",
      permissions: ["media.microphoneLevel"],
      connects: [{ host: "tracks.example.org", why: "Previews" }],
    });
    const chess = release({ id: "chess", name: "Chess", version: "0.3.0" });
    api.mockImplementation(async (path) => {
      if (path === base) {
        return {
          activities: [
            { ...music, install: "music-install", enabled: true },
            { ...chess, install: "chess-install", enabled: false },
          ],
          hosting: true,
        };
      }
      throw new Error("Unexpected activity request " + path);
    });
    render(<ActivitiesPage />);

    const musicRow = await screen.findByRole("row", { name: /Music/ });
    expect(within(musicRow).getByText("1.0.0")).toBeTruthy();
    expect(within(musicRow).getByRole("link", { name: "example/games" }).getAttribute("href")).toBe(url);
    expect(within(musicRow).getByText("Głośność mikrofonu")).toBeTruthy();
    expect(within(musicRow).getByText("tracks.example.org")).toBeTruthy();
    expect(within(musicRow).getByText("Nie sprawdzono")).toBeTruthy();
    expect(within(musicRow).getByRole("switch").getAttribute("aria-checked")).toBe("true");
    expect(within(musicRow).getByText("Włączona")).toBeTruthy();

    const chessRow = row("Chess");
    expect(within(chessRow).getByText("Brak")).toBeTruthy();
    expect(within(chessRow).getByRole("switch").getAttribute("aria-checked")).toBe("false");
    expect(within(chessRow).getByText("Wyłączona")).toBeTruthy();
  });

  it("says so when nothing is installed, and when the instance cannot serve activities", async () => {
    api.mockResolvedValue({ activities: [], hosting: false });
    render(<ActivitiesPage />);

    expect(await screen.findByText(/Nie zainstalowano jeszcze żadnej aktywności/)).toBeTruthy();
    expect(screen.getByRole("alert").textContent).toContain("domenę aktywności");
    expect((screen.getByRole("button", { name: "Sprawdź aktualizacje" }) as HTMLButtonElement).disabled).toBe(true);
  });

  it("reviews and installs just the selected activity from a multi-activity release", async () => {
    const music = release({ id: "music", name: "Music", version: "1.0.0" });
    const cards = {
      ...release({
        id: "cards",
        name: "Cards",
        version: "2.0.0",
        permissions: ["media.cameraTile"],
        connects: [{ host: "art.example.org", why: "Card illustrations" }],
      }),
      artwork: { icon: "data:image/png;base64,AQ==", cover: "data:image/webp;base64,Ag==" },
    };
    const choice = ({ release: page, manifest, bytes }: typeof music) => ({
      release: page,
      id: manifest.id,
      name: manifest.name,
      version: manifest.version,
      bytes,
    });
    // A separate installation from this source already exists and stays enabled.
    const library = [{ ...music, install: "music-install", enabled: true }];
    api.mockImplementation(async (path, options = {}) => {
      if (path === base) return { activities: structuredClone(library), hosting: true };
      if (path === base + "/discover") return { source: url, entries: [choice(music), choice(cards)] };
      if (path === base + "/review") return cards;
      if (path === base + "/install") {
        library.push({ ...cards, install: "cards-install", enabled: true });
        return cards;
      }
      if (path === base + "/cards-install/manage") {
        expect(options.body).toEqual({ action: "disable" });
        library[1]!.enabled = false;
        return { ok: true };
      }
      throw new Error("Unexpected activity request " + path);
    });
    const user = userEvent.setup();
    render(<ActivitiesPage />);
    await screen.findByRole("row", { name: /Music/ });

    await user.click(screen.getByRole("button", { name: "Dodaj aktywność" }));
    const dialog = screen.getByRole("dialog");
    await user.type(within(dialog).getByRole("textbox"), url);
    await user.click(within(dialog).getByRole("button", { name: "Znajdź aktywności" }));

    // A repository with several activities is not read until one is chosen.
    const pick = await within(dialog).findByRole("button", { name: /Cards.*2\.0\.0/ });
    expect(within(dialog).getByRole("button", { name: /Music.*1\.0\.0/ })).toBeTruthy();
    expect(sent("/review")).toHaveLength(0);
    expect(within(dialog).queryByRole("button", { name: "Zainstaluj i włącz" })).toBeNull();

    await user.click(pick);
    expect(await within(dialog).findByText("art.example.org")).toBeTruthy();
    expect(dialog.querySelector('img[src="data:image/png;base64,AQ=="]')).not.toBeNull();
    expect(dialog.querySelector('img[src="data:image/webp;base64,Ag=="]')).not.toBeNull();
    expect(within(dialog).getByText(/Pokazywanie kamery/)).toBeTruthy();
    expect(within(dialog).getByText("Card illustrations")).toBeTruthy();

    await user.click(within(dialog).getByRole("button", { name: "Zainstaluj i włącz" }));
    const installed = await screen.findByRole("row", { name: /Cards/ });
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(screen.getByRole("alert").textContent).toBe("Zainstalowano i włączono aktywność Cards 2.0.0.");
    expect(sent("/review")).toEqual([{ url: cards.release, id: "cards", update: undefined }]);
    expect(sent("/install")).toEqual([
      { url: cards.release, id: "cards", digest: cards.digest, revision: cards.revision, update: undefined },
    ]);

    await user.click(within(installed).getByRole("switch", { name: "Włącz aktywność Cards" }));
    await waitFor(() => expect(within(row("Cards")).getByRole("switch").getAttribute("aria-checked")).toBe("false"));
    expect(library.map((entry) => [entry.install, entry.enabled])).toEqual([
      ["music-install", true],
      ["cards-install", false],
    ]);
  });

  it("keeps the reason in the dialog when a repository offers nothing", async () => {
    api.mockImplementation(async (path) => {
      if (path === base) return { activities: [], hosting: true };
      if (path === base + "/discover") {
        throw new CockpitApiError(400, "Bad request", { activity: ["No matching stable activity release."] });
      }
      throw new Error("Unexpected activity request " + path);
    });
    const user = userEvent.setup();
    render(<ActivitiesPage />);
    await screen.findByText(/Nie zainstalowano jeszcze/);

    await user.click(screen.getByRole("button", { name: "Dodaj aktywność" }));
    const dialog = screen.getByRole("dialog");
    await user.type(within(dialog).getByRole("textbox"), url);
    await user.click(within(dialog).getByRole("button", { name: "Znajdź aktywności" }));

    expect((await within(dialog).findByRole("alert")).textContent).toBe("No matching stable activity release.");
    expect(sent("/review")).toHaveLength(0);
  });

  it("checks every installation in turn, shows what changed in an update and installs it", async () => {
    const cards = release({
      id: "cards",
      name: "Cards",
      version: "2.0.0",
      permissions: ["media.cameraTile", "media.gifs"],
      connects: [{ host: "art.example.org", why: "Card illustrations" }],
    });
    const next = release({
      id: "cards",
      name: "Cards",
      version: "2.1.0",
      permissions: ["media.cameraTile", "media.microphoneLevel"],
      connects: [
        { host: "art.example.org", why: "Card illustrations" },
        { host: "stats.example.org", why: "Rankings" },
      ],
    });
    const music = release({ id: "music", name: "Music", version: "1.0.0" });
    const library = [
      { ...cards, install: "cards-install", enabled: true },
      { ...music, install: "music-install", enabled: true },
    ];
    let reading = 0;
    api.mockImplementation(async (path, options = {}) => {
      const body = options.body as { update?: string } | undefined;
      if (path === base) return { activities: structuredClone(library), hosting: true };
      if (path === base + "/review") {
        // The server reads two releases at once at most, so the page asks for one at a time.
        expect(++reading).toBe(1);
        await Promise.resolve();
        reading--;
        return body?.update === "cards-install" ? next : { current: true };
      }
      if (path === base + "/install") {
        library[0] = { ...next, install: "cards-install", enabled: true };
        return next;
      }
      throw new Error("Unexpected activity request " + path);
    });
    const user = userEvent.setup();
    render(<ActivitiesPage />);
    await screen.findByRole("row", { name: /Cards/ });

    await user.click(screen.getByRole("button", { name: "Sprawdź aktualizacje" }));
    expect(await within(row("Cards")).findByText("Dostępna 2.1.0")).toBeTruthy();
    expect(await within(row("Music")).findByText("Aktualna")).toBeTruthy();
    expect(sent("/review")).toEqual([
      { url, id: "cards", update: "cards-install" },
      { url, id: "music", update: "music-install" },
    ]);

    // The update that was found opens without being read a second time.
    await user.click(within(row("Cards")).getByRole("button", { name: "Aktualizuj" }));
    const dialog = screen.getByRole("dialog");
    expect(within(dialog).getByText("2.0.0 → 2.1.0")).toBeTruthy();
    const line = (text: RegExp | string) => within(dialog).getByText(text).closest("li")!;
    expect(within(line(/Odczyt głośności mikrofonu/)).getByText("Dodano")).toBeTruthy();
    expect(within(line(/GIF-ów z GIPHY/)).getByText("Usunięto")).toBeTruthy();
    expect(within(line(/Pokazywanie kamery/)).queryByText("Dodano")).toBeNull();
    expect(within(line("stats.example.org")).getByText("Nowy lub zmieniony")).toBeTruthy();
    expect(within(line("art.example.org")).queryByText("Nowy lub zmieniony")).toBeNull();
    expect(sent("/review")).toHaveLength(2);

    await user.click(within(dialog).getByRole("button", { name: "Aktualizuj" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(sent("/install")).toEqual([
      { url: next.release, id: "cards", digest: next.digest, revision: next.revision, update: "cards-install" },
    ]);
    expect(screen.getByRole("alert").textContent).toBe("Zaktualizowano aktywność Cards do wersji 2.1.0.");
    const updated = row("Cards");
    expect(within(updated).getByText("2.1.0")).toBeTruthy();
    expect(within(updated).getByText("Aktualna")).toBeTruthy();
    expect(within(updated).queryByRole("button", { name: "Aktualizuj" })).toBeNull();
  });

  it("stops checking at GitHub's rate limit and names the failure in the row", async () => {
    const library = ["cards", "music"].map((id) => ({
      ...release({ id, name: id === "cards" ? "Cards" : "Music", version: "1.0.0" }),
      install: `${id}-install`,
      enabled: true,
    }));
    api.mockImplementation(async (path) => {
      if (path === base) return { activities: library, hosting: true };
      if (path === base + "/review") throw new CockpitApiError(429, "GitHub rate limit reached.");
      throw new Error("Unexpected activity request " + path);
    });
    const user = userEvent.setup();
    render(<ActivitiesPage />);
    await screen.findByRole("row", { name: /Cards/ });

    await user.click(screen.getByRole("button", { name: "Sprawdź aktualizacje" }));

    const failed = row("Cards");
    expect(await within(failed).findByText("Nie udało się sprawdzić")).toBeTruthy();
    expect(within(failed).getByText("GitHub rate limit reached.")).toBeTruthy();
    expect(within(row("Music")).getByText("Nie sprawdzono")).toBeTruthy();
    expect(sent("/review")).toHaveLength(1);
  });

  it("uninstalls from the row's menu only after the dialog confirms it", async () => {
    const library = [{ ...release({ id: "cards", name: "Cards", version: "2.0.0" }), install: "cards-install", enabled: true }];
    api.mockImplementation(async (path, options = {}) => {
      if (path === base) return { activities: structuredClone(library), hosting: true };
      if (path === base + "/cards-install/manage") {
        expect(options.body).toEqual({ action: "uninstall" });
        library.length = 0;
        return { ok: true };
      }
      throw new Error("Unexpected activity request " + path);
    });
    const user = userEvent.setup();
    render(<ActivitiesPage />);
    await screen.findByRole("row", { name: /Cards/ });

    await user.click(screen.getByRole("button", { name: "Akcje dla aktywności Cards" }));
    await user.click(screen.getByRole("menuitem", { name: "Odinstaluj" }));
    const dialog = screen.getByRole("dialog", { name: "Odinstalować aktywność Cards?" });
    expect(sent("/cards-install/manage")).toHaveLength(0);

    await user.click(within(dialog).getByRole("button", { name: "Odinstaluj" }));
    expect(await screen.findByText(/Nie zainstalowano jeszcze/)).toBeTruthy();
    expect(screen.queryByRole("row", { name: /Cards/ })).toBeNull();
    expect(screen.getByRole("alert").textContent).toBe("Odinstalowano aktywność Cards.");
  });
});
