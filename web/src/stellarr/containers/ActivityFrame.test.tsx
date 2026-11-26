import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";

import { FakeRealtimeClient } from "@/test/fakes";
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

it("takes a mounted camera image away and stops asking for pictures once the frame goes", async () => {
  vi.useFakeTimers();
  try {
    const tile = vi.fn(async (_action: Record<string, unknown>, _signal: AbortSignal) => ({ a: "data:image/gif;base64,R0lGODlhAQABAAAAACw=" }));
    const view = render(
      <ActivityFrame src="https://game.test/play" title="Charades" permissions={["media.cameraTile"]}
        room={async () => ({})} tile={tile} onLost={vi.fn()} />,
    );
    const frame = screen.getByTitle<HTMLIFrameElement>("Charades");
    const port = { postMessage: vi.fn(), close: vi.fn(), onmessage: null as null | ((event: unknown) => void) };
    window.dispatchEvent(Object.assign(new Event("message"), {
      source: frame.contentWindow, origin: "https://game.test", data: "stellarr:media:v1", ports: [port],
    }));
    port.onmessage?.({ data: { op: "mount", id: 1, bounds: [0, 0, 10, 10, 100, 100], participant: "a" } });
    const image = view.container.querySelector(".activity-viewport img");
    expect(image).not.toBeNull();
    await act(async () => { await vi.advanceTimersByTimeAsync(250); });
    expect(tile).toHaveBeenCalledOnce();

    view.unmount();

    expect(image?.parentNode).toBeNull();
    expect(tile.mock.calls[0]?.[1].aborted).toBe(true);
    await act(async () => { await vi.advanceTimersByTimeAsync(1000); });
    expect(tile).toHaveBeenCalledOnce();
  } finally {
    vi.useRealTimers();
  }
});

it("routes installed session traffic to the hub and forwards events only to their frame", async () => {
  const realtime = new FakeRealtimeClient();
  const room = vi.fn(async () => ({ roomHub: true }));
  const liveRoom = vi.fn(async (_action: Record<string, unknown>, _signal: AbortSignal) => ({}));
  const mounted = render(<ActivityFrame src="https://game.test/play" title="Board" permissions={[]}
    room={room} liveRoom={liveRoom} realtime={realtime} tile={async () => ({})} onLost={vi.fn()} />);
  const frame = screen.getByTitle<HTMLIFrameElement>("Board");
  const port = { postMessage: vi.fn(), close: vi.fn(), onmessage: null as null | ((event: unknown) => Promise<void>) };
  window.dispatchEvent(Object.assign(new Event("message"), { source: frame.contentWindow, origin: "https://game.test",
    data: "stellarr:dev-connect:v1", ports: [port] }));
  await port.onmessage!({ data: { id: 1, input: { op: "open", roomEvents: true } } });
  expect(port.postMessage.mock.lastCall?.[0].value.roomEvents).toBe(true);
  await port.onmessage!({ data: { id: 2, input: { op: "read", roomEvents: true } } });
  expect(liveRoom).toHaveBeenCalledWith(expect.objectContaining({ op: "read" }), expect.any(AbortSignal));
  const id = port.postMessage.mock.calls.length;
  realtime.push("ActivityRoomUpdate", { frame: "another-frame", settled: "deal" });
  expect(port.postMessage).toHaveBeenCalledTimes(id);
  const update = { frame: String(liveRoom.mock.calls[0]![0].frame), settled: "deal" };
  realtime.push("ActivityRoomUpdate", update);
  expect(port.postMessage).toHaveBeenLastCalledWith({ update });
  await port.onmessage!({ data: { id: 3, input: { op: "loadProgress" } } });
  expect(room).toHaveBeenCalledTimes(2);
  mounted.unmount();
  port.postMessage.mockClear();
  realtime.push("ActivityRoomUpdate", update);
  expect(port.postMessage).not.toHaveBeenCalled();
});
