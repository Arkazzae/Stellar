import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";

import { FakeRealtimeClient } from "@/test/fakes";
import { ActivityFrame, type FrameParts } from "./ActivityFrame";

let locale = "pl";
vi.mock("@/state/LanguageProvider", () => ({
  useLanguage: () => ({ locale, t: (key: string) => key }),
}));
// A search that GIPHY answers with nothing, so the page owes its mark.
vi.mock("@/lib/activities/gifs", () => ({
  createActivityGifs: ({ onUse }: { onUse(): void }) => ({
    search: async () => {
      onUse();
      return { items: [], offset: 0, total: 0 };
    },
    load: async () => Promise.reject(Object.assign(new Error("No such GIF."), { code: "notFound" })),
  }),
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

it("plays WebRTC tiles in the parent and drops a late capture after consent is withdrawn", async () => {
  const watchers = new Map<string, (stream: MediaStream | null) => void>();
  const transport = {
    publish: vi.fn(), dispose: vi.fn(),
    watch: (id: string, show: (stream: MediaStream | null) => void) => {
      watchers.set(id, show);
      return () => { watchers.delete(id); show(null); };
    },
  };
  let giveCamera!: (stream: MediaStream) => void;
  const capture = vi.fn(() => new Promise<MediaStream>(resolve => { giveCamera = resolve; }));
  const mediaDevices = Object.getOwnPropertyDescriptor(navigator, "mediaDevices");
  Object.defineProperty(navigator, "mediaDevices", { configurable: true, value: { getUserMedia: capture } });
  const track = { stop: vi.fn() };
  const stream = { getTracks: () => [track] } as unknown as MediaStream;
  const ask = vi.fn(async () => true);
  try {
    const view = render(<ActivityFrame src="https://game.test/play" title="Camera" permissions={["media.cameraTile"]}
      room={async () => ({})} camera={() => transport} ask={ask} onLost={vi.fn()} />);
    const frame = screen.getByTitle<HTMLIFrameElement>("Camera");
    const port = { postMessage: vi.fn(), close: vi.fn(), onmessage: null as null | ((event: unknown) => void) };
    window.dispatchEvent(Object.assign(new Event("message"), {
      source: frame.contentWindow, origin: "https://game.test", data: "stellarr:media:v1", ports: [port],
    }));
    const send = (data: object) => port.onmessage?.({ data });
    send({ op: "mount", id: 1, participant: "alice", bounds: [0, 0, 10, 10, 100, 100] });
    watchers.get("alice")!(stream);
    const video = view.container.querySelector("video")!;
    expect(video.srcObject).toBe(stream);
    expect(capture).not.toHaveBeenCalled();
    await act(async () => { send({ op: "open", id: 2, kind: "cameraTile" }); });
    expect(ask).toHaveBeenCalledOnce();
    expect(capture).toHaveBeenCalledOnce();
    send({ op: "stop", kind: "cameraTile" });
    await act(async () => { giveCamera(stream); });
    expect(track.stop).toHaveBeenCalledOnce();
    expect(transport.publish).not.toHaveBeenCalled();
    expect(port.postMessage.mock.calls.flat()).not.toContain(stream);
    view.unmount();
    expect(watchers.size).toBe(0);
    expect(transport.dispose).toHaveBeenCalledOnce();
    expect(video.srcObject).toBeNull();
  } finally {
    if (mediaDevices) Object.defineProperty(navigator, "mediaDevices", mediaDevices);
    else Reflect.deleteProperty(navigator, "mediaDevices");
  }
});

it("shows GIPHY's mark once the activity has used GIFs, and not before", async () => {
  render(<ActivityFrame src="https://game.test/play" title="Prompts" permissions={["media.gifs"]}
    room={async () => ({})} onLost={vi.fn()} />);
  const frame = screen.getByTitle<HTMLIFrameElement>("Prompts");
  const port = { postMessage: vi.fn(), close: vi.fn(), onmessage: null as null | ((event: unknown) => void) };
  window.dispatchEvent(Object.assign(new Event("message"), {
    source: frame.contentWindow, origin: "https://game.test", data: "stellarr:media:v1", ports: [port],
  }));
  expect(screen.queryByAltText("gif.poweredBy")).toBeNull();

  await act(async () => {
    port.onmessage?.({ data: { op: "gifSearch", id: 1, query: "cat" } });
    await vi.waitFor(() => expect(port.postMessage).toHaveBeenCalledWith(
      { op: "reply", id: 1, value: { items: [], offset: 0, total: 0 } }, []));
  });

  expect(screen.getByAltText("gif.poweredBy")).toBeVisible();
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

it("hands the bar each device the activity opens, stops one from there, and passes the page's volume on", async () => {
  const capture = vi.fn(() => new Promise<MediaStream>(() => {}));
  const mediaDevices = Object.getOwnPropertyDescriptor(navigator, "mediaDevices");
  Object.defineProperty(navigator, "mediaDevices", { configurable: true, value: { getUserMedia: capture } });
  const transport = { publish: vi.fn(), dispose: vi.fn(), watch: () => () => {} };
  const bar = ({ devices, stopDevice }: FrameParts) => <div>
    {devices.map(kind => <button key={kind} type="button" onClick={() => stopDevice(kind)}>{"stop " + kind}</button>)}
  </div>;
  const props = { src: "https://game.test/play", title: "Charades", permissions: ["media.cameraTile"],
    room: async () => ({}), camera: () => transport, ask: async () => true, onLost: vi.fn(), bar };
  try {
    const view = render(<ActivityFrame {...props} volume={0.8} />);
    expect(screen.queryByRole("slider")).toBeNull();
    const frame = screen.getByTitle<HTMLIFrameElement>("Charades");
    const port = { postMessage: vi.fn(), close: vi.fn(), onmessage: null as null | ((event: unknown) => void) };
    act(() => {
      window.dispatchEvent(Object.assign(new Event("message"), {
        source: frame.contentWindow, origin: "https://game.test", data: "stellarr:media:v1", ports: [port],
      }));
    });
    expect(port.postMessage).toHaveBeenCalledWith({ op: "volume", value: 0.8 });
    view.rerender(<ActivityFrame {...props} volume={0} />);
    expect(port.postMessage).toHaveBeenLastCalledWith({ op: "volume", value: 0 });

    await act(async () => { port.onmessage?.({ data: { op: "open", id: 3, kind: "cameraTile" } }); });
    act(() => { screen.getByRole("button", { name: "stop cameraTile" }).click(); });

    expect(port.postMessage).toHaveBeenCalledWith({ op: "ended", kind: "cameraTile", reason: "stopped" });
    expect(screen.queryByRole("button", { name: "stop cameraTile" })).toBeNull();
  } finally {
    if (mediaDevices) Object.defineProperty(navigator, "mediaDevices", mediaDevices);
    else Reflect.deleteProperty(navigator, "mediaDevices");
  }
});
