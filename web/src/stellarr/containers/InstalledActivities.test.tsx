import { useState } from "react";

import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";

import { request } from "@/lib/api/transport";
import { InstalledActivities, type VoiceActivityBarControl } from "./InstalledActivities";

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
  function take(path: "list" | "launch" | "leave") {
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

function VoiceLibrary() {
  const [open, setOpen] = useState(false);
  return <>
    <button onClick={() => setOpen(true)}>Activities</button>
    <InstalledActivities channel="voice" pickerOpen={open} onClosePicker={() => setOpen(false)} />
  </>;
}
const openPicker = () => fireEvent.click(screen.getByRole("button", { name: "Activities" }));
const start = () => fireEvent.click(screen.getByRole("button", { name: /Static game/ }));
async function poll() {
  await act(async () => { await vi.advanceTimersByTimeAsync(10000); });
}

async function openStatic() {
  const api = server();
  render(<VoiceLibrary />);
  openPicker();
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
  openPicker();
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

it("keeps all ten games inside the voice dialog, supports search and distinguishes start, join and unsupported devices", async () => {
  const api = server();
  render(<VoiceLibrary />);
  const description = "A game for the whole channel.";
  const games = Array.from({ length: 10 }, (_, i) => ({
    ...entry, install: String(i), name: "Game " + i, running: i === 0, supported: i !== 9,
    artwork: { cover: "/cover.png", icon: "/icon.png" }, description, devices: ["tablet"], seats: 8,
  }));
  expect(request).not.toHaveBeenCalled();
  expect(screen.getAllByRole("button")).toHaveLength(1);
  const opener = screen.getByRole("button", { name: "Activities" });
  opener.focus();
  openPicker();
  await api.take("list").reply(games);
  const dialog = within(screen.getByRole("dialog", { name: "stage.voicePickerTitle" }));
  expect(dialog.getAllByRole("listitem")).toHaveLength(10);
  const join = dialog.getByRole("button", { name: "Game 0 · stage.joinActivity" });
  expect(join).toBeEnabled();
  expect(join).toHaveAccessibleDescription(description);
  expect(within(join).getAllByRole("presentation")).toHaveLength(2);
  expect(dialog.getByRole("button", { name: "Game 1 · stage.beginActivity" })).toBeEnabled();
  expect(dialog.getByRole("button", { name: "Game 9 · tablet" })).toBeDisabled();

  const search = dialog.getByRole("searchbox", { name: "stage.searchActivities" });
  fireEvent.change(search, { target: { value: "Game 7" } });
  expect(dialog.getAllByRole("listitem")).toHaveLength(1);
  expect(dialog.getByRole("button", { name: "Game 7 · stage.beginActivity" })).toBeEnabled();

  fireEvent.change(search, { target: { value: "no-such-game" } });
  expect(dialog.getByRole("status")).toHaveTextContent("stage.noActivityMatch");
  fireEvent.click(dialog.getByRole("button", { name: "stage.clearActivitySearch" }));
  expect(dialog.getAllByRole("listitem")).toHaveLength(10);

  fireEvent.keyDown(document, { key: "Escape" });
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(opener).toHaveFocus();
});

it("joins a channel activity directly from the voice bar control and notifies the server on leave", async () => {
  const api = server();
  const state: { bar: VoiceActivityBarControl | null } = { bar: null };
  render(<InstalledActivities channel="voice" onVoiceBarChange={next => { state.bar = next; }} />);
  expect(state.bar?.live).toBeNull();
  expect(state.bar?.joined).toBe(false);

  await act(async () => { state.bar?.join(); });
  await api.take("list").reply([entry]);
  const directLaunch = api.take("launch");
  expect(directLaunch.input).toEqual({ device: "desktop" });
  await directLaunch.reply({ ...launchReply("bar-lease"), install: "game" });
  await api.take("list").reply([entry]);

  expect(state.bar?.joined).toBe(true);
  expect(state.bar?.live).toEqual({ install: "game", name: entry.name, icon: null });
  expect(screen.getByTitle(entry.name)).toHaveAttribute("src", "https://game.test/bar-lease");

  await act(async () => { state.bar?.leave(); });
  const leaveReq = api.take("leave");
  expect(leaveReq.input).toEqual({ install: "game", access: "bar-lease" });
  await leaveReq.reply({ ok: true, running: false });
  expect(state.bar?.joined).toBe(false);
  expect(state.bar?.live).toBeNull();
});

it("falls back to joining the active voice session when another member wins a launch race with 409", async () => {
  const api = server();
  render(<VoiceLibrary />);
  openPicker();
  const other = { ...entry, install: "other", name: "Other game", running: false };
  await api.take("list").reply([{ ...entry, running: false }, other]);
  fireEvent.click(screen.getByRole("button", { name: /Other game/ }));
  await api.take("list").reply([entry, other]);
  await api.take("launch").reply("Conflict", 409);
  const fallback = api.take("launch");
  expect(fallback.input).toEqual({ device: "desktop" });
  await fallback.reply({ ...launchReply("joined-lease"), install: "game" });
  await api.take("list").reply([entry, other]);
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(screen.getByTitle(entry.name)).toHaveAttribute("src", "https://game.test/joined-lease");
});

it("shows an empty voice library inside the dialog", async () => {
  const api = server();
  render(<VoiceLibrary />);
  openPicker();
  expect(screen.getByRole("status")).toHaveTextContent("common.loading");
  await api.take("list").reply([]);
  expect(screen.getByRole("status")).toHaveTextContent("stage.noVoiceActivities");
});

it("returns to a running frame and keeps it when a switch is dismissed or fails", async () => {
  const api = await openStatic();
  const frame = screen.getByTitle(entry.name);
  openPicker();
  fireEvent.click(screen.getByRole("button", { name: /stage.returnToActivity/ }));
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(screen.getByTitle(entry.name)).toBe(frame);

  const games = [entry, { ...entry, install: "other", name: "Other game" }];
  await poll();
  await api.take("list").reply(games);
  openPicker();
  fireEvent.click(screen.getByRole("button", { name: /Other game/ }));
  const stale = api.take("launch");
  fireEvent.click(screen.getByRole("button", { name: "common.close" }));
  await stale.reply(launchReply("other-lease"));
  expect(screen.getByTitle(entry.name)).toBe(frame);

  openPicker();
  fireEvent.click(screen.getByRole("button", { name: /Other game/ }));
  await api.take("launch").reply(null, 403);
  expect(within(screen.getByRole("dialog")).getByRole("alert")).toBeInTheDocument();
  expect(screen.getByTitle(entry.name)).toBe(frame);
});
