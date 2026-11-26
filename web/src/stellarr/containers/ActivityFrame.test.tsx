import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";

import { ActivityFrame } from "./ActivityFrame";

let locale = "pl";
vi.mock("@/state/LanguageProvider", () => ({
  useLanguage: () => ({ locale, t: (key: string) => key }),
}));
afterEach(() => { cleanup(); vi.unstubAllGlobals(); });

it.each(["https://game.test/play", "/_activity/lease/index.html"])("passes live context, players and usable portrait URLs to %s", async src => {
  locale = "pl";
  let coarse = true;
  vi.stubGlobal("matchMedia", () => ({ matches: coarse }));
  vi.stubGlobal("screen", { width: 390, height: 844 });
  const player = { id: "a", name: "Alice", username: "alice", avatar: null as string | null, connected: true };
  const second = { ...player, id: "b", username: "bob", name: "Bob" };
  let version = 1;
  const room = vi.fn(async () => ({ participants: [{ ...player }, { ...second }], version }));
  const props = { src, title: "Poll", permissions: [], room, tile: async () => ({}), onLost: vi.fn() };
  const view = render(<ActivityFrame {...props} />);
  const frame = screen.getByTitle<HTMLIFrameElement>("Poll");
  const port = { postMessage: vi.fn(), close: vi.fn(), onmessage: null as null | ((event: unknown) => Promise<void>) };
  const event = new Event("message");
  const url = new URL(frame.src);
  const nonce = new URLSearchParams(url.hash.slice(1)).get("stellarr");
  Object.assign(event, { source: frame.contentWindow, origin: nonce ? "null" : url.origin,
    data: nonce ? { type: "stellarr:dev-connect:v1", nonce } : "stellarr:dev-connect:v1", ports: [port] });
  window.dispatchEvent(event);
  const request = async (op: string) => {
    await act(async () => { await port.onmessage!({ data: { id: 1, input: { op } } }); });
    return port.postMessage.mock.lastCall?.[0].value;
  };
  expect(await request("open")).toMatchObject({ locale: "pl", device: "phone" });
  expect(screen.getByText("Alice")).toBeVisible();
  locale = "en";
  vi.stubGlobal("screen", { width: 1024, height: 768 });
  player.name = "Alicja";
  second.name = "Alicja";
  player.avatar = "/_stellarr/avatars/a/picture";
  version++;
  view.rerender(<ActivityFrame {...props} />);
  const update = await request("read");
  expect(update).toMatchObject({ locale: "en", device: "tablet" });
  expect(update.participants[0].avatar).toBe(new URL("_stellarr/avatars/a/picture", url).href);
  expect(screen.getByText("Alicja (@alice)")).toBeVisible();
  expect(screen.getByText("Alicja (@bob)")).toBeVisible();
  expect(view.container.querySelector("li img")).toHaveAttribute("src", "/assets/picture");
  coarse = false;
  expect(await request("read")).toMatchObject({ device: "desktop" });
  expect(screen.getByTitle("Poll")).toBe(frame);
});
