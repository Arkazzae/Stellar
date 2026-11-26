import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it, vi } from "vitest";

import type * as CockpitApiModule from "../lib/api";
import { cockpitRequest } from "../lib/api";
import { selectPolish } from "../test/locale";
import { ActivitiesPage } from "./activities-page";

selectPolish();
vi.mock("../lib/api", async importOriginal => ({
  ...await importOriginal<typeof CockpitApiModule>(),
  cockpitRequest: vi.fn(),
}));

it("reviews and installs just the selected activity from a multi-activity release", async () => {
  const url = "https://github.com/example/games";
  const release = url + "/releases/tag/cards/v2.0.0";
  const base = "/api/cockpit/activities";
  const entries = [
    { release: url + "/releases/tag/music/v1.0.0", id: "music", name: "Music", version: "1.0.0", bytes: 5600 },
    { release, id: "cards", name: "Cards", version: "2.0.0", bytes: 8300 },
  ];
  const cards = {
    artwork: { icon: "data:image/png;base64,AQ==", cover: "data:image/webp;base64,Ag==" },
    source: url, release, digest: "d".repeat(64), revision: "e".repeat(40), bytes: 8300,
    manifest: { ...entries[1], description: "Play cards", permissions: ["media.cameraTile"], connects: [{ host: "art.example.org", why: "Card illustrations" }] },
  };
  // A separate installation from this source already exists and stays enabled.
  const library = [{ ...cards, install: "music-install", enabled: true, manifest: { ...cards.manifest, ...entries[0] } }];
  const api = vi.mocked(cockpitRequest);
  api.mockImplementation(async (path, options = {}) => {
    if (path === base) return { activities: structuredClone(library), hosting: true };
    if (path === base + "/discover") return { source: url, release, entries };
    if (path === base + "/review") return cards;
    if (path === base + "/install") { library.push({ ...cards, install: "cards-install", enabled: true }); return cards; }
    if (path === base + "/cards-install/manage") {
      expect(options.body).toEqual({ action:"disable" });
      library[1]!.enabled = false;
      return { ok:true };
    }
    throw new Error("Unexpected activity request " + path);
  });
  const user = userEvent.setup();
  render(<ActivitiesPage />);
  await user.type(screen.getByRole("textbox"),url);
  await user.click(screen.getByRole("button",{name:"Dodaj aktywność"}));
  const choice = await screen.findByRole("button",{name:/Cards · 2.0.0/});
  expect(screen.getByRole("button",{name:/Music · 1.0.0/})).toBeTruthy();
  expect(api.mock.calls.filter(([path])=>path === base + "/review")).toHaveLength(0);
  expect(screen.queryByRole("button",{name:"Zainstaluj i włącz"})).toBeNull();
  await user.click(choice);
  await screen.findByText("art.example.org");
  expect(document.querySelector('img[src="data:image/png;base64,AQ=="]')).not.toBeNull();
  expect(document.querySelector('img[src="data:image/webp;base64,Ag=="]')).not.toBeNull();
  expect(screen.getByText(/Pokazywanie kamery/)).toBeTruthy();
  expect(screen.getByText(/Card illustrations/)).toBeTruthy();
  await user.click(screen.getByRole("button",{name:"Zainstaluj i włącz"}));
  await waitFor(()=>expect(screen.getAllByRole("button",{name:"Wyłącz"})).toHaveLength(2));
  expect(api.mock.calls.filter(([path])=>path === base + "/review").map(([,options])=>options?.body))
    .toEqual([{url:release,id:"cards",update:undefined}]);
  expect(api.mock.calls.filter(([path])=>path === base + "/install").map(([,options])=>options?.body))
    .toEqual([{url:release,id:"cards",digest:cards.digest,revision:cards.revision,update:undefined}]);
  await user.click(screen.getAllByRole("button",{name:"Wyłącz"})[1]!);
  await screen.findByRole("button",{name:"Włącz"});
  expect(library.map(entry => [entry.install,entry.enabled])).toEqual([["music-install",true],["cards-install",false]]);
});
