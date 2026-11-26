import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";

import type { ActivitySession } from "@/state/hooks";
import { resetStubs, setStubs, stubs } from "@/test/hookStubs";

import { ActivityPanel } from "./ActivityHost";

vi.mock("@/state/hooks", async () => (await import("@/test/hookStubs")).hooksModule);

const session: ActivitySession = {
  place: "voice-1",
  install: "synctube",
  access: "lease-one",
  src: "https://synctube.games.test/",
  title: "SyncTube",
  icon: null,
  permissions: [],
  youtube: false,
};

function inside(host: string) {
  setStubs({
    activities: {
      voice: session,
      live: new Map([
        [
          "voice-1",
          {
            install: "synctube",
            name: "SyncTube",
            icon: null,
            host,
            participants: [host, host === "u-maja" ? "u-ada" : "u-maja"],
            seats: null,
          },
        ],
      ]),
    },
  });
  return render(<ActivityPanel slot="voice" session={session} target={null} />);
}

beforeEach(() => {
  resetStubs();
  setStubs({
    directory: {
      members: [
        { id: "u-maja", username: "maja", displayName: "Maja", presence: "online" },
        { id: "u-ada", username: "ada", displayName: "Ania Mazur", presence: "online" },
      ],
    },
  });
});

it("gives the host a menu to change the activity or end it for everyone", () => {
  inside("u-maja");
  const bar = within(screen.getByRole("region", { name: "SyncTube" }));

  expect(bar.getByText("2 os. w środku · prowadzisz")).toBeInTheDocument();
  fireEvent.click(bar.getByRole("button", { name: "Więcej" }));
  expect(bar.getByText("Gdy wyjdziesz, prowadzenie przejmie Ania Mazur, a SyncTube trwa dalej.")).toBeInTheDocument();
  fireEvent.click(bar.getByRole("button", { name: "Zmień aktywność" }));
  expect(stubs.activities.openPicker).toHaveBeenCalledWith({ scope: "voice", channelId: "voice-1", change: true });

  fireEvent.click(bar.getByRole("button", { name: "Więcej" }));
  fireEvent.click(bar.getByRole("button", { name: "Zakończ dla wszystkich" }));
  expect(stubs.activities.end).toHaveBeenCalledWith("voice-1");
});

it("lets anyone else only leave, and says who hosts", () => {
  inside("u-ada");
  const bar = within(screen.getByRole("region", { name: "SyncTube" }));

  expect(bar.getByText("2 os. w środku · prowadzi Ania Mazur")).toBeInTheDocument();
  expect(bar.queryByRole("button", { name: "Więcej" })).toBeNull();
  fireEvent.click(bar.getByRole("button", { name: "Wyjdź z aktywności" }));
  expect(stubs.activities.leave).toHaveBeenCalledWith("voice");
  expect(stubs.activities.end).not.toHaveBeenCalled();
});

it("asks with the app's card before an activity gets the camera, and refuses on no", async () => {
  const camera = { ...session, permissions: ["media.cameraTile"] };
  setStubs({ activities: { voice: camera, live: new Map() } });
  render(<ActivityPanel slot="voice" session={camera} target={null} />);
  const frame = screen.getByTitle<HTMLIFrameElement>("SyncTube");
  const port = { postMessage: vi.fn(), close: vi.fn(), onmessage: null as null | ((event: unknown) => void) };
  const hello = Object.assign(new Event("message"), {
    source: frame.contentWindow,
    origin: "https://synctube.games.test",
    data: "stellarr:media:v1",
    ports: [port],
  });
  act(() => {
    window.dispatchEvent(hello);
  });
  await act(async () => {
    port.onmessage?.({ data: { op: "open", kind: "cameraTile", id: 7 } });
  });

  const question = screen.getByRole("dialog", { name: "Pokazać kamerę w SyncTube?" });
  expect(question).toHaveTextContent("Zobaczą ją tylko osoby w tej aktywności.");
  expect(port.postMessage).not.toHaveBeenCalledWith(expect.objectContaining({ op: "reply" }));
  await act(async () => {
    fireEvent.click(within(question).getByRole("button", { name: "Nie pokazuj" }));
  });

  expect(port.postMessage).toHaveBeenCalledWith({ op: "reply", id: 7, error: "Permission denied." });
  expect(screen.queryByRole("dialog")).toBeNull();
});
