import { act, render } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";

import {
  activitiesApi,
  ActivityRequestError,
  PERSONAL,
  type ActivityLaunch,
  type ActivityLibrary,
} from "@/lib/activities/api";
import type * as Api from "@/lib/activities/api";
import type { VoiceActivitySummary } from "@/lib/realtime/types";

import { ActivitiesProvider, useActivitiesContext, type ActivitiesValue } from "./ActivitiesProvider";

vi.mock("@/lib/activities/api", async (importOriginal) => {
  const actual = await importOriginal<typeof Api>();
  return {
    ...actual,
    activitiesApi: {
      list: vi.fn(),
      launch: vi.fn(),
      leave: vi.fn(async () => {}),
      end: vi.fn(async () => {}),
      act: vi.fn(),
      live: vi.fn(),
    },
  };
});

const world = vi.hoisted(() => ({
  call: { channelId: null as string | null, connection: "idle", error: null as string | null },
  handlers: new Map<string, (payload: unknown) => void>(),
  ui: {
    joinVoiceChannel: (() => undefined) as (channelId: string) => void,
    returnToChannel: (() => undefined) as () => void,
    openActivity: (() => undefined) as () => void,
  },
}));

vi.mock("./DirectoryProvider", () => ({
  useDirectoryContext: () => ({ server: { id: "srv-orbit", name: "Orbit" } }),
}));
vi.mock("./SessionProvider", () => ({
  useSessionContext: () => ({ connection: "connected", user: { id: "u-maja" } }),
}));
vi.mock("./VoiceProvider", () => ({
  useVoiceContext: () => ({
    channelId: world.call.channelId,
    state: { connection: world.call.connection },
    error: world.call.error,
  }),
}));
vi.mock("./UiProvider", () => ({ useUiContext: () => world.ui }));
const realtime = vi.hoisted(() => ({
  on: (event: string, handler: (payload: unknown) => void) => {
    world.handlers.set(event, handler);
    return () => world.handlers.delete(event);
  },
}));
vi.mock("./PortsProvider", () => ({ useOptionalRealtime: () => realtime }));

const api = vi.mocked(activitiesApi);

const seat = (access: string, install = "synctube"): ActivityLaunch => ({
  install,
  access,
  src: `https://${access}.games.test/`,
  title: install === "synctube" ? "SyncTube" : "Buzzer",
  permissions: [],
  host: "u-maja",
  embedHosts: [],
});
const synctube = { install: "synctube", name: "SyncTube", icon: "/icon.png", seats: null };
const summary: VoiceActivitySummary = {
  install: "synctube",
  name: "SyncTube",
  icon: "/icon.png",
  host: "u-ada",
  participants: ["u-ada"],
  seats: 4,
};
const library: ActivityLibrary = {
  activities: [],
  live: summary,
  owner: null,
  viewerIsOwner: false,
  voiceOnly: [],
};

function mount() {
  const seen: { value: ActivitiesValue | null } = { value: null };
  function Probe() {
    seen.value = useActivitiesContext();
    return null;
  }
  const tree = () => (
    <ActivitiesProvider>
      <Probe />
    </ActivitiesProvider>
  );
  const view = render(tree());
  return {
    get value() {
      if (seen.value === null) throw new Error("not rendered");
      return seen.value;
    },
    rerender: () => view.rerender(tree()),
  };
}

const flush = (ms = 0) => act(async () => {
  await vi.advanceTimersByTimeAsync(ms);
});

beforeEach(() => {
  vi.useFakeTimers();
  world.call = { channelId: null, connection: "idle", error: null };
  world.handlers.clear();
  world.ui = { joinVoiceChannel: vi.fn(), returnToChannel: vi.fn(), openActivity: vi.fn() };
  api.live.mockResolvedValue(new Map());
  api.list.mockResolvedValue({ ...library, activities: [] });
});
afterEach(() => {
  vi.useRealTimers();
  vi.clearAllMocks();
});

it("joins the call first and the activity once the call connects, waiting out a lagging seat", async () => {
  const activities = mount();
  act(() => activities.value.join("voice-1"));

  expect(world.ui.joinVoiceChannel).toHaveBeenCalledWith("voice-1");
  expect(activities.value.joining).toBe("voice-1");
  expect(api.launch).not.toHaveBeenCalled();

  api.launch
    .mockRejectedValueOnce(new ActivityRequestError(403, null, "Forbidden"))
    .mockResolvedValueOnce(seat("lease-one"));
  world.call = { channelId: "voice-1", connection: "connected", error: null };
  activities.rerender();
  await flush();
  expect(api.launch).toHaveBeenLastCalledWith("voice-1", { device: "desktop" });
  await flush(500);

  expect(api.launch).toHaveBeenCalledTimes(2);
  expect(activities.value.voice).toMatchObject({ place: "voice-1", access: "lease-one" });
  expect(activities.value.joining).toBeNull();
});

it("stops joining at once, with the call's reason, when the call itself cannot be joined", async () => {
  const activities = mount();
  act(() => activities.value.join("voice-1"));
  expect(activities.value.joining).toBe("voice-1");

  world.call = { channelId: "voice-1", connection: "failed", error: "Kanał jest pełny." };
  activities.rerender();

  expect(activities.value.joining).toBeNull();
  expect(activities.value.joinFailure).toEqual({ channelId: "voice-1", text: "Kanał jest pełny." });
  await flush(30_000);
  expect(api.launch).not.toHaveBeenCalled();
});

it("says why a join was refused instead of retrying a full activity", async () => {
  world.call = { channelId: "voice-1", connection: "connected", error: null };
  const activities = mount();
  api.launch.mockRejectedValueOnce(new ActivityRequestError(409, "full", "Full"));

  act(() => activities.value.join("voice-1"));
  await flush();

  expect(api.launch).toHaveBeenCalledOnce();
  expect(activities.value.voice).toBeNull();
  expect(activities.value.joinFailure).toEqual({
    channelId: "voice-1",
    text: "Komplet. Spróbuj, gdy ktoś wyjdzie.",
  });
});

it("renews the seat and ends the session only when the server no longer knows it", async () => {
  world.call = { channelId: "voice-1", connection: "connected", error: null };
  const activities = mount();
  api.launch.mockResolvedValueOnce(seat("lease-one"));
  await act(() => activities.value.launch("voice-1", synctube));

  expect(api.list).toHaveBeenLastCalledWith(
    "voice-1",
    { device: "desktop", lease: { install: "synctube", access: "lease-one" } },
    expect.any(AbortSignal),
  );
  await flush(10_000);
  expect(api.list).toHaveBeenCalledTimes(2);

  // A swap says "nothing" first: the seat is asked about at once, and kept.
  act(() => world.handlers.get("VoiceActivityChanged")?.({ channelId: "voice-1", activity: null }));
  await flush();
  expect(api.list).toHaveBeenCalledTimes(3);
  expect(activities.value.voice).not.toBeNull();

  api.list.mockRejectedValueOnce(new ActivityRequestError(410, null, "Gone"));
  await flush(10_000);
  expect(activities.value.voice).toBeNull();
  expect(activities.value.ended).toEqual({ place: "voice-1", name: "SyncTube" });
});

it("knows what runs on the server's channels and follows the hub", async () => {
  api.live.mockResolvedValueOnce(new Map([["voice-1", summary]]));
  const activities = mount();
  await flush();

  expect(api.live).toHaveBeenCalledWith("srv-orbit", expect.any(AbortSignal));
  expect(activities.value.live.get("voice-1")).toEqual(summary);

  const joined = { ...summary, participants: ["u-ada", "u-kenji"] };
  act(() => world.handlers.get("VoiceActivityChanged")?.({ channelId: "voice-1", activity: joined }));
  expect(activities.value.live.get("voice-1")).toEqual(joined);
  act(() => world.handlers.get("VoiceActivityChanged")?.({ channelId: "voice-1", activity: null }));
  expect(activities.value.live.has("voice-1")).toBe(false);
});

it("lets the personal activity running make way for the next one, once that one starts", async () => {
  const activities = mount();
  const buzzer = { install: "buzzer", name: "Buzzer", icon: null, seats: null };
  api.launch
    .mockResolvedValueOnce(seat("lease-one"))
    .mockRejectedValueOnce(new ActivityRequestError(500, null, "Broken"))
    .mockResolvedValueOnce(seat("lease-two", "buzzer"));

  await act(() => activities.value.launch(PERSONAL, synctube));
  expect(world.ui.openActivity).toHaveBeenCalledOnce();
  await act(() => expect(activities.value.launch(PERSONAL, buzzer)).rejects.toThrow("Broken"));
  expect(api.leave).not.toHaveBeenCalled();
  expect(activities.value.personal).toMatchObject({ install: "synctube", access: "lease-one" });

  await act(() => activities.value.launch(PERSONAL, buzzer));

  expect(api.leave).toHaveBeenCalledWith(PERSONAL, { install: "synctube", access: "lease-one" });
  expect(api.launch).toHaveBeenLastCalledWith(PERSONAL, { install: "buzzer", device: "desktop" });
  expect(activities.value.personal).toMatchObject({ install: "buzzer", access: "lease-two" });
});
