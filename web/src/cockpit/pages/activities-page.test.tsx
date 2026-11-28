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

interface Kept {
  installedAt?: string | null;
  updatedAt?: string | null;
  check?: { checkedAt: string; version: string | null; release: string | null; error: string | null } | null;
}

/** An installation as the library lists it. */
function installed(version: ReturnType<typeof release>, install: string, enabled = true, kept: Kept = {}) {
  return { ...version, install, enabled, installedAt: null, updatedAt: null, check: null, ...kept };
}

/** A look the server made at noon on 14 March 2025, a day that is never today. */
const looked = (found: Partial<NonNullable<Kept["check"]>> = {}) => ({
  checkedAt: "2025-03-14T12:00:00Z",
  version: null,
  release: null,
  error: null,
  ...found,
});

// A block body: Vitest would run a returned mock as the test's teardown.
beforeEach(() => {
  api.mockReset();
});

describe("ActivitiesPage", () => {
  it("lists what is installed with its version and its date, the last update check, source, access and state", async () => {
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
            installed(music, "music-install", true, {
              installedAt: "2026-11-01T12:00:00Z",
              updatedAt: "2026-11-20T12:00:00Z",
              check: looked(),
            }),
            installed(chess, "chess-install", false, {
              check: looked({ version: "0.4.0", release: `${url}/releases/tag/chess/v0.4.0` }),
            }),
          ],
          hosting: true,
          checkHours: 12,
        };
      }
      throw new Error("Unexpected activity request " + path);
    });
    render(<ActivitiesPage />);

    const musicRow = await screen.findByRole("row", { name: /Music/ });
    expect(within(musicRow).getByText("1.0.0")).toBeTruthy();
    // The day the current version came, an update here, in full on hover.
    const since = within(musicRow).getByText("20 lis 2026");
    expect(since.getAttribute("title")).toMatch(/^Zaktualizowano 20 lis 2026/);
    expect(within(musicRow).getByRole("link", { name: "example/games" }).getAttribute("href")).toBe(url);
    expect(within(musicRow).getByText("Głośność mikrofonu")).toBeTruthy();
    expect(within(musicRow).getByText("tracks.example.org")).toBeTruthy();
    // The server's own look, before anybody asked.
    expect(within(musicRow).getByText("Aktualna")).toBeTruthy();
    expect(within(musicRow).getByText(/^Sprawdzono 14 mar/)).toBeTruthy();
    expect(screen.getByText("Aktualizacje sprawdzane automatycznie co 12 godz.")).toBeTruthy();
    expect(within(musicRow).getByRole("switch").getAttribute("aria-checked")).toBe("true");
    expect(within(musicRow).getByText("Włączona")).toBeTruthy();

    const chessRow = row("Chess");
    expect(within(chessRow).getByText("0.3.0").nextElementSibling).toBeNull();
    expect(within(chessRow).getByText("Dostępna 0.4.0")).toBeTruthy();
    expect(within(chessRow).getByRole("button", { name: "Aktualizuj" })).toBeTruthy();
    expect(within(chessRow).getByText("Brak")).toBeTruthy();
    expect(within(chessRow).getByRole("switch").getAttribute("aria-checked")).toBe("false");
    expect(within(chessRow).getByText("Wyłączona")).toBeTruthy();
  });

  it("says so when nothing is installed, and when the instance cannot serve activities", async () => {
    api.mockResolvedValue({ activities: [], hosting: false, checkHours: 12 });
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
    const library = [installed(music, "music-install")];
    api.mockImplementation(async (path, options = {}) => {
      if (path === base) return { activities: structuredClone(library), hosting: true, checkHours: 12 };
      if (path === base + "/discover") return { source: url, entries: [choice(music), choice(cards)] };
      if (path === base + "/review") return cards;
      if (path === base + "/install") {
        library.push(installed(cards, "cards-install"));
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
    const added = await screen.findByRole("row", { name: /Cards/ });
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(screen.getByRole("alert").textContent).toBe("Zainstalowano i włączono aktywność Cards 2.0.0.");
    expect(sent("/review")).toEqual([{ url: cards.release, id: "cards", update: undefined }]);
    expect(sent("/install")).toEqual([
      { url: cards.release, id: "cards", digest: cards.digest, revision: cards.revision, update: undefined },
    ]);

    await user.click(within(added).getByRole("switch", { name: "Włącz aktywność Cards" }));
    await waitFor(() => expect(within(row("Cards")).getByRole("switch").getAttribute("aria-checked")).toBe("false"));
    expect(library.map((entry) => [entry.install, entry.enabled])).toEqual([
      ["music-install", true],
      ["cards-install", false],
    ]);
  });

  it("keeps the reason in the dialog when a repository offers nothing", async () => {
    api.mockImplementation(async (path) => {
      if (path === base) return { activities: [], hosting: true, checkHours: 12 };
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

  it("has the server look at every installation in turn, then reviews and installs the update it found", async () => {
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
    const library = [installed(cards, "cards-install"), installed(music, "music-install")];
    let looking = 0;
    api.mockImplementation(async (path, options = {}) => {
      if (path === base) return { activities: structuredClone(library), hosting: true, checkHours: 12 };
      const asked = path.match(/^\/api\/cockpit\/activities\/(.+)\/check$/);
      if (asked) {
        // The server reads two releases at once at most, so the page asks for one at a time.
        expect(++looking).toBe(1);
        await Promise.resolve();
        looking--;
        const entry = library.find((candidate) => candidate.install === asked[1])!;
        entry.check = entry === library[0] ? looked({ version: "2.1.0", release: next.release }) : looked();
        return entry.check;
      }
      if (path === base + "/review") return next;
      if (path === base + "/install") {
        // The server carries the look that found this version over to it.
        library[0] = installed(next, "cards-install", true, { check: looked() });
        return next;
      }
      throw new Error("Unexpected activity request " + path + " " + JSON.stringify(options.body));
    });
    const user = userEvent.setup();
    render(<ActivitiesPage />);
    await screen.findByRole("row", { name: /Cards/ });
    expect(within(row("Cards")).getByText("Nie sprawdzono")).toBeTruthy();

    await user.click(screen.getByRole("button", { name: "Sprawdź aktualizacje" }));
    expect(await within(row("Cards")).findByText("Dostępna 2.1.0")).toBeTruthy();
    expect(await within(row("Music")).findByText("Aktualna")).toBeTruthy();
    expect(api.mock.calls.map(([called]) => called).filter((called) => called.endsWith("/check"))).toEqual([
      base + "/cards-install/check",
      base + "/music-install/check",
    ]);
    expect(sent("/review")).toHaveLength(0);

    // The release is read, for the review, only when the owner opens it.
    await user.click(within(row("Cards")).getByRole("button", { name: "Aktualizuj" }));
    const dialog = await screen.findByRole("dialog");
    expect(sent("/review")).toEqual([{ url, id: "cards", update: "cards-install" }]);
    expect(within(dialog).getByText("2.0.0 → 2.1.0")).toBeTruthy();
    const line = (text: RegExp | string) => within(dialog).getByText(text).closest("li")!;
    expect(within(line(/Odczyt głośności mikrofonu/)).getByText("Dodano")).toBeTruthy();
    expect(within(line(/GIF-ów z GIPHY/)).getByText("Usunięto")).toBeTruthy();
    expect(within(line(/Pokazywanie kamery/)).queryByText("Dodano")).toBeNull();
    expect(within(line("stats.example.org")).getByText("Nowy lub zmieniony")).toBeTruthy();
    expect(within(line("art.example.org")).queryByText("Nowy lub zmieniony")).toBeNull();

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

  it("looks again when the release a check found is gone by the time it is opened", async () => {
    const cards = release({ id: "cards", name: "Cards", version: "2.0.0" });
    const library = [
      installed(cards, "cards-install", true, {
        check: looked({ version: "2.1.0", release: `${url}/releases/tag/cards/v2.1.0` }),
      }),
    ];
    api.mockImplementation(async (path) => {
      if (path === base) return { activities: structuredClone(library), hosting: true, checkHours: 12 };
      if (path === base + "/review") return { current: true };
      if (path === base + "/cards-install/check") {
        library[0]!.check = looked();
        return library[0]!.check;
      }
      throw new Error("Unexpected activity request " + path);
    });
    const user = userEvent.setup();
    render(<ActivitiesPage />);

    await user.click(await within(await screen.findByRole("row", { name: /Cards/ })).findByRole("button", { name: "Aktualizuj" }));

    expect(await within(row("Cards")).findByText("Aktualna")).toBeTruthy();
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(screen.getByRole("alert").textContent).toBe("Aktywność Cards jest aktualna.");
  });

  it("stops checking at GitHub's rate limit, and the row keeps the reason the server stored", async () => {
    const library = ["cards", "music"].map((id) =>
      installed(release({ id, name: id === "cards" ? "Cards" : "Music", version: "1.0.0" }), `${id}-install`),
    );
    api.mockImplementation(async (path) => {
      if (path === base) return { activities: structuredClone(library), hosting: true, checkHours: 12 };
      if (path === base + "/cards-install/check") {
        library[0]!.check = looked({ error: "GitHub rate limit reached." });
        throw new CockpitApiError(429, "GitHub rate limit reached.");
      }
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
    expect(screen.getByRole("alert").textContent).toBe("GitHub rate limit reached.");
    expect(api.mock.calls.filter(([called]) => called.endsWith("/check"))).toHaveLength(1);
  });

  it("uninstalls from the row's menu only after the dialog confirms it", async () => {
    const library = [installed(release({ id: "cards", name: "Cards", version: "2.0.0" }), "cards-install")];
    api.mockImplementation(async (path, options = {}) => {
      if (path === base) return { activities: structuredClone(library), hosting: true, checkHours: 12 };
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
