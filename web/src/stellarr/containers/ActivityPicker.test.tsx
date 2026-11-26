import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";

import { ConfirmProvider } from "@/components/primitives/ConfirmDialog";
import {
  activitiesApi,
  ActivityRequestError,
  type ActivityEntry,
  type ActivityLibrary,
} from "@/lib/activities/api";
import type * as Api from "@/lib/activities/api";
import type { VoiceActivitySummary } from "@/lib/realtime/types";
import { resetStubs, setStubs, stubs } from "@/test/hookStubs";

import { ActivityPicker } from "./ActivityPicker";

vi.mock("@/state/hooks", async () => (await import("@/test/hookStubs")).hooksModule);
vi.mock("@/lib/activities/api", async (importOriginal) => ({
  ...(await importOriginal<typeof Api>()),
  activitiesApi: { list: vi.fn() },
}));

const list = vi.mocked(activitiesApi.list);

const entry = (install: string, name: string, extra: Partial<ActivityEntry> = {}): ActivityEntry => ({
  install,
  name,
  version: "1.0.0",
  description: `${name} for the channel.`,
  icon: null,
  cover: null,
  running: false,
  supported: true,
  devices: null,
  seats: null,
  permissions: [],
  ...extra,
});
const library = (patch: Partial<ActivityLibrary> = {}): ActivityLibrary => ({
  activities: [
    entry("synctube", "SyncTube"),
    entry("buzzer", "Buzzer", { seats: 8 }),
    entry("jump", "Voice Jump", { supported: false, devices: ["phone"], permissions: ["media.microphoneLevel"] }),
  ],
  live: null,
  owner: null,
  viewerIsOwner: false,
  voiceOnly: [],
  ...patch,
});
const running: VoiceActivitySummary = {
  install: "synctube",
  name: "SyncTube",
  icon: null,
  host: "u-maja",
  participants: ["u-maja", "u-ada"],
  seats: null,
};

function open(picker = { scope: "voice" as const, channelId: "voice-1", change: false }) {
  setStubs({ activities: { picker } });
  return render(
    <ConfirmProvider>
      <ActivityPicker />
    </ConfirmProvider>,
  );
}
const settle = () => act(async () => {});
const dialog = () => within(screen.getByRole("dialog", { name: "Aktywności" }));

beforeEach(() => {
  resetStubs();
  setStubs({
    session: { connection: "connected" },
    directory: {
      channelName: () => "voice",
      members: [{ id: "u-ada", username: "ada", displayName: "Ania Mazur", presence: "online" }],
    },
  });
});
afterEach(() => {
  vi.useRealTimers();
  vi.clearAllMocks();
});

it("starts the picked activity, and says on the card why the server refused another", async () => {
  list.mockResolvedValueOnce(library());
  vi.mocked(stubs.activities.launch)
    .mockResolvedValueOnce(undefined)
    .mockRejectedValueOnce(new ActivityRequestError(409, "full", "Full"));
  open();
  await settle();

  expect(list).toHaveBeenCalledWith("voice-1", { device: "desktop" }, expect.any(AbortSignal));
  expect(dialog().getByText("Dla osób na kanale voice. Pozostali dołączą jednym kliknięciem.")).toBeInTheDocument();
  const jump = dialog().getByRole("button", { name: "Voice Jump Nie działa na komputerze" });
  expect(jump).toHaveAttribute("aria-disabled", "true");
  expect(within(jump).getByText("Poziom mikrofonu")).toBeInTheDocument();
  fireEvent.click(jump);
  expect(stubs.activities.launch).not.toHaveBeenCalled();

  fireEvent.click(dialog().getByRole("button", { name: "SyncTube Uruchom" }));
  await settle();
  expect(stubs.activities.launch).toHaveBeenCalledWith(
    "voice-1",
    { install: "synctube", name: "SyncTube", icon: null, seats: null },
    {},
  );

  const buzzer = dialog().getByRole("button", { name: "Buzzer Uruchom" });
  expect(within(buzzer).getByText("Do 8 osób")).toBeInTheDocument();
  fireEvent.click(buzzer);
  await settle();
  expect(buzzer).toHaveAccessibleDescription(/Komplet\. Spróbuj, gdy ktoś wyjdzie\./);
});

it("asks the host before swapping the running activity, and swaps only on yes", async () => {
  list.mockResolvedValue(library({ live: running }));
  setStubs({ activities: { live: new Map([["voice-1", running]]) } });
  open({ scope: "voice", channelId: "voice-1", change: true });
  await settle();

  expect(dialog().getByText("Prowadzisz. W środku też Ania Mazur.")).toBeInTheDocument();
  const swap = () => fireEvent.click(dialog().getByRole("button", { name: "Buzzer Zamień" }));

  swap();
  await settle();
  const question = screen.getByRole("dialog", { name: "Zamienić SyncTube na Buzzer?" });
  expect(question).toHaveTextContent("SyncTube zakończy się u wszystkich na kanale voice, a w jej miejsce uruchomi się Buzzer.");
  expect(question).toHaveTextContent("W środku Ania Mazur");
  fireEvent.click(within(question).getByRole("button", { name: "Anuluj" }));
  await settle();
  expect(stubs.activities.launch).not.toHaveBeenCalled();

  swap();
  await settle();
  fireEvent.click(screen.getByRole("button", { name: "Zamień" }));
  await settle();
  expect(stubs.activities.launch).toHaveBeenCalledWith(
    "voice-1",
    { install: "buzzer", name: "Buzzer", icon: null, seats: 8 },
    { replace: true },
  );
});

it("tries again on its own, further apart each time, while the server does not answer", async () => {
  vi.useFakeTimers();
  list
    .mockRejectedValueOnce(new ActivityRequestError(503, null, "Unavailable"))
    .mockRejectedValueOnce(new ActivityRequestError(503, null, "Unavailable"))
    .mockResolvedValueOnce(library());
  open();
  await settle();

  expect(dialog().getByRole("status")).toHaveTextContent("Serwer nie odpowiada");
  expect(dialog().getByRole("status")).toHaveTextContent("Kolejna próba za 3 s. Błąd 503.");
  await act(async () => {
    await vi.advanceTimersByTimeAsync(3000);
  });
  expect(list).toHaveBeenCalledTimes(2);
  expect(dialog().getByRole("status")).toHaveTextContent("Kolejna próba za 6 s.");

  fireEvent.click(dialog().getByRole("button", { name: "Spróbuj teraz" }));
  await settle();
  expect(list).toHaveBeenCalledTimes(3);
  expect(dialog().queryByText("Serwer nie odpowiada")).toBeNull();
  expect(dialog().getByRole("button", { name: "SyncTube Uruchom" })).toBeInTheDocument();
});

it.each([
  [true, "Aktywności to gry i wspólne oglądanie, które osoby na kanale uruchamiają razem. Dodajesz je w Cockpicie.", "Otwórz Cockpit"],
  [false, "Dodaje je właściciel instancji.", "Napisz"],
] as const)("with nothing to pick (owner: %s) says who can add activities", async (viewerIsOwner, text, action) => {
  list.mockResolvedValueOnce(
    library({
      activities: [],
      viewerIsOwner,
      owner: { id: "u-owner", name: "Owner", avatar: null },
    }),
  );
  open();
  await settle();

  expect(dialog().getByText("Na tej instancji nie ma jeszcze aktywności")).toBeInTheDocument();
  expect(dialog().getByText(text)).toBeInTheDocument();
  if (action === "Napisz") {
    expect(dialog().getByText("Właściciel instancji")).toBeInTheDocument();
    fireEvent.click(dialog().getByRole("button", { name: "Napisz" }));
    expect(stubs.friends.messagePerson).toHaveBeenCalledWith("u-owner");
    expect(stubs.activities.closePicker).toHaveBeenCalled();
  } else {
    expect(dialog().getByRole("link", { name: action })).toHaveAttribute("href", "/cockpit/activities");
    expect(dialog().queryByText("Właściciel instancji")).toBeNull();
  }
});
