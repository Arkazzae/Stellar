import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";

import { request } from "@/lib/api/transport";
import { InstalledActivities } from "./InstalledActivities";

vi.mock("@/lib/api/transport", () => ({ request: vi.fn() }));
vi.mock("@/state/LanguageProvider", () => ({
  useLanguage: () => ({ t: (key: string) => key }),
}));

const entry = { install: "game", name: "Static game", supported: true, running: true, version: "1.0.0" };
const launchReply = (access: string) => ({
  access, src: "https://game.test/" + access, title: entry.name, permissions: [],
});

function server() {
  const pending: Array<{
    path: string; input: unknown; resolve: (response: Response) => void; reject: (error: Error) => void;
  }> = [];
  // The transport can deliver a response even after our component abandons it.
  vi.mocked(request).mockImplementation((path, init) => new Promise((resolve, reject) => {
    pending.push({ path, input: JSON.parse(String(init.body)), resolve, reject });
  }));
  function take(path: "list" | "launch") {
    const call = pending.find(item => item.path.endsWith("/" + path));
    if (!call) throw new Error("Missing " + path + " request");
    pending.splice(pending.indexOf(call), 1);
    return {
      input: call.input,
      reset: () => act(async () => { call.reject(new Error("Connection reset")); }),
      reply: (body: unknown, status = 200) => act(async () => {
        call.resolve(Response.json(body, { status }));
      }),
    };
  }
  return { take };
}

const start = () => fireEvent.click(screen.getByRole("button", { name: /Static game/ }));
async function poll() {
  await act(async () => { await vi.advanceTimersByTimeAsync(10000); });
}

async function openStatic() {
  const api = server();
  render(<InstalledActivities channel="voice" />);
  await api.take("list").reply([{ ...entry, running: false }]);
  start();
  await api.take("list").reply([{ ...entry, running: false }]);
  await api.take("launch").reply(launchReply("lease-one"));
  const heartbeat = api.take("list");
  expect(heartbeat.input).toEqual({ device: "desktop", lease: { install: "game", access: "lease-one" } });
  await heartbeat.reply([entry]);
  expect(screen.getByTitle(entry.name)).toHaveAttribute("src", "https://game.test/lease-one");
  return api;
}

beforeEach(() => vi.useFakeTimers());
afterEach(() => { cleanup(); vi.useRealTimers(); vi.clearAllMocks(); });

it.each([
  ["removed from the library", [], 200],
  ["room no longer running", [{ ...entry, running: false }], 200],
  ["voice access revoked", null, 403],
  ["lease replaced by another launch", null, 410],
] as const)("closes a static frame when %s", async (_case, body, status) => {
  const api = await openStatic();
  await poll();
  await api.take("list").reply(body, status);
  expect(screen.queryByTitle(entry.name)).toBeNull();
  expect(screen.getByRole("alert")).toBeInTheDocument();
  expect(vi.mocked(request).mock.calls.some(([path]) => path.endsWith("/room"))).toBe(false);
});

it("keeps a static frame across temporary errors and renews it beyond the 60-second lease", async () => {
  const api = await openStatic();
  await poll();
  await api.take("list").reply(null, 503);
  expect(screen.getByTitle(entry.name)).toBeInTheDocument();
  await poll();
  await api.take("list").reset();
  expect(screen.getByTitle(entry.name)).toBeInTheDocument();
  for (let i = 0; i < 7; i++) {
    await poll();
    const heartbeat = api.take("list");
    expect(heartbeat.input).toMatchObject({ lease: { access: "lease-one" } });
    await heartbeat.reply([entry]);
  }
  expect(screen.getByTitle(entry.name)).toBeInTheDocument();
  expect(screen.queryByRole("alert")).toBeNull();
});

it("ignores an earlier lease rejection after the same game is opened again", async () => {
  const api = await openStatic();
  await poll();
  const stale = api.take("list");
  fireEvent.click(screen.getByRole("button", { name: "stage.leaveActivity" }));
  await api.take("list").reply([entry]);
  start();
  const beforeLaunch = api.take("list");
  await api.take("launch").reply(launchReply("lease-two"));
  await api.take("list").reply([entry]);
  await stale.reply(null, 410);
  await beforeLaunch.reply([]);
  expect(screen.getByTitle(entry.name)).toHaveAttribute("src", "https://game.test/lease-two");
  expect(screen.queryByRole("alert")).toBeNull();
});

it("discards a late successful launch after a list response invalidates the pending launch", async () => {
  const api = server();
  render(<InstalledActivities />);
  await api.take("list").reply([entry]);
  start();
  const stale = api.take("launch");
  await api.take("list").reply([]);
  expect(screen.queryByRole("button", { name: /Static game/ })).toBeNull();
  await api.take("list").reply([entry]);
  start();
  await api.take("list").reply([entry]);
  await api.take("launch").reply(launchReply("new-lease"));
  await api.take("list").reply([entry]);
  await stale.reply(launchReply("stale-lease"));
  expect(screen.getByTitle(entry.name)).toHaveAttribute("src", "https://game.test/new-lease");
});
