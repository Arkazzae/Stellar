import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";

import { ConfirmProvider } from "@/components/primitives/ConfirmDialog";
import { setDockAlways } from "@/lib/activities/dockPreference";
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
  embedHosts: [],
};

/** The places the call's bar keeps for the activity: its own column and the end of the view's controls. */
function callBar() {
  const start = document.body.appendChild(document.createElement("div"));
  const end = document.body.appendChild(document.createElement("span"));
  return { start, end };
}

function inside(host: string) {
  const bars = callBar();
  setStubs({
    activities: {
      bars,
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
  render(
    <ConfirmProvider>
      <ActivityPanel slot="voice" session={session} target={null} />
    </ConfirmProvider>,
  );
  return bars;
}

afterEach(() => {
  document.body.replaceChildren();
  setDockAlways(false);
  Reflect.deleteProperty(document, "fullscreenElement");
});

beforeEach(() => {
  resetStubs();
  setStubs({
    directory: {
      channelName: () => "voice",
      members: [
        { id: "u-maja", username: "maja", displayName: "Maja", presence: "online" },
        { id: "u-ada", username: "ada", displayName: "Ania Mazur", presence: "online" },
      ],
    },
  });
});

it("puts the activity into the call's bar, with no bar of its own over the frame", () => {
  const { start, end } = inside("u-maja");
  const panel = within(screen.getByRole("region", { name: "SyncTube" }));

  expect(within(start).getByText("2 os. w środku · prowadzisz")).toBeInTheDocument();
  expect(within(start).getByRole("button", { name: "Ustawienia aktywności" })).toBeInTheDocument();
  expect(within(end).getByRole("button", { name: "Wyjdź z aktywności" })).toBeInTheDocument();
  expect(panel.queryByRole("button")).toBeNull();
});

it("gives the host a menu to change the activity or, once asked, end it for everyone", async () => {
  const { start, end } = inside("u-maja");
  const bar = within(end);

  expect(within(start).getByText("2 os. w środku · prowadzisz")).toBeInTheDocument();
  fireEvent.click(bar.getByRole("button", { name: "Więcej" }));
  expect(bar.getByText("Gdy wyjdziesz, prowadzenie przejmie Ania Mazur, a SyncTube trwa dalej.")).toBeInTheDocument();
  fireEvent.click(bar.getByRole("button", { name: "Zmień aktywność" }));
  expect(stubs.activities.openPicker).toHaveBeenCalledWith({ scope: "voice", channelId: "voice-1", change: true });

  fireEvent.click(bar.getByRole("button", { name: "Więcej" }));
  await act(async () => {
    fireEvent.click(bar.getByRole("button", { name: "Zakończ dla wszystkich" }));
  });
  const question = screen.getByRole("dialog", { name: "Zakończyć SyncTube dla wszystkich?" });
  expect(question).toHaveTextContent("Aktywność zamknie się u 2 os. na kanale voice.");
  expect(stubs.activities.end).not.toHaveBeenCalled();
  await act(async () => {
    fireEvent.click(within(question).getByRole("button", { name: "Zakończ" }));
  });
  expect(stubs.activities.end).toHaveBeenCalledWith("voice-1");
});

it("lets anyone else only leave, and says who hosts", () => {
  const { start, end } = inside("u-ada");
  const bar = within(end);

  expect(within(start).getByText("2 os. w środku · prowadzi Ania Mazur")).toBeInTheDocument();
  expect(bar.queryByRole("button", { name: "Więcej" })).toBeNull();
  fireEvent.click(bar.getByRole("button", { name: "Wyjdź z aktywności" }));
  expect(stubs.activities.leave).toHaveBeenCalledWith("voice");
  expect(stubs.activities.end).not.toHaveBeenCalled();
});

it("asks with the app's card before an activity gets the camera, and refuses on no", async () => {
  const camera = { ...session, permissions: ["media.cameraTile"] };
  setStubs({ activities: { voice: camera, live: new Map() } });
  render(
    <ConfirmProvider>
      <ActivityPanel slot="voice" session={camera} target={null} />
    </ConfirmProvider>,
  );
  // Without the call's bar in view the controls stand under the frame, named as it is.
  expect(screen.getByRole("button", { name: "Wyjdź z aktywności" })).toBeInTheDocument();
  const frame = document.querySelector<HTMLIFrameElement>('iframe[title="SyncTube"]')!;
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

/** Connects to the frame the way the SDK does, and returns the page's end of the bridge. */
function connectFrame(title: string) {
  const frame = screen.getByTitle<HTMLIFrameElement>(title);
  const port = { postMessage: vi.fn(), close: vi.fn(), onmessage: null as null | ((event: unknown) => Promise<void>) };
  act(() => {
    window.dispatchEvent(Object.assign(new Event("message"), {
      source: frame.contentWindow,
      origin: "https://synctube.games.test",
      data: "stellarr:dev-connect:v1",
      ports: [port],
    }));
  });
  const ask = async (dock: string) => {
    await act(async () => {
      await port.onmessage?.({ data: { id: 1, input: { op: "shell", dock } } });
    });
  };
  return { port, ask };
}

function goFullscreen(panel: HTMLElement) {
  Object.defineProperty(document, "fullscreenElement", { configurable: true, get: () => panel });
  act(() => {
    document.dispatchEvent(new Event("fullscreenchange"));
  });
}

it("lets a full-screen activity hold the dock's edge, and keeps a handle that still brings the dock up", async () => {
  inside("u-maja");
  const panel = screen.getByRole("region", { name: "SyncTube" });
  const { port, ask } = connectFrame("SyncTube");

  await ask("hold");
  expect(port.postMessage).toHaveBeenCalledWith({ id: 1, value: {} });
  goFullscreen(panel);

  expect(panel).toHaveAttribute("data-held");
  expect(panel).not.toHaveAttribute("data-dock");
  expect(panel.querySelector(".act-wake")).toBeNull();
  expect(port.postMessage).toHaveBeenLastCalledWith({
    shell: { dock: { fullscreen: true, visible: false, held: true, height: 0 } },
  });
  const dock = within(screen.getByRole("group", { name: "Dok aktywności" }));
  expect(dock.getByRole("button", { name: "Wyjdź z aktywności" })).toBeInTheDocument();

  fireEvent.click(screen.getByRole("button", { name: "Pokaż dok" }));
  expect(panel).toHaveAttribute("data-dock", "shown");
  expect(screen.queryByRole("button", { name: "Pokaż dok" })).toBeNull();

  await ask("release");
  expect(panel).not.toHaveAttribute("data-held");
  expect(panel.querySelector(".act-wake")).not.toBeNull();
});

it("lets the player's own answer beat the activity's hold", async () => {
  setDockAlways(true);
  inside("u-maja");
  const panel = screen.getByRole("region", { name: "SyncTube" });
  const { port, ask } = connectFrame("SyncTube");

  await ask("hold");
  goFullscreen(panel);

  expect(panel).not.toHaveAttribute("data-held");
  expect(panel).toHaveAttribute("data-dock", "shown");
  expect(panel.querySelector(".act-wake")).not.toBeNull();
  expect(screen.queryByRole("button", { name: "Pokaż dok" })).toBeNull();
  expect(port.postMessage).toHaveBeenLastCalledWith({
    shell: { dock: expect.objectContaining({ fullscreen: true, visible: true, held: false }) },
  });
});

it("answers an unknown request of the dock with an error, and never sends it to the session", async () => {
  inside("u-maja");
  const { port, ask } = connectFrame("SyncTube");

  await ask("pin");

  expect(port.postMessage).toHaveBeenCalledWith({ id: 1, error: "Unknown session operation.", status: 400 });
});

it("keeps the dock up on a touch screen, tells the activity so, and leaves only the handle while it holds", async () => {
  const media = window.matchMedia;
  window.matchMedia = ((query: string) => ({
    matches: query === "(hover: none)", media: query, onchange: null,
    addEventListener: () => {}, removeEventListener: () => {}, addListener: () => {}, removeListener: () => {}, dispatchEvent: () => false,
  })) as unknown as typeof window.matchMedia;
  try {
    inside("u-maja");
    const panel = screen.getByRole("region", { name: "SyncTube" });
    const { port, ask } = connectFrame("SyncTube");
    goFullscreen(panel);

    expect(panel).toHaveAttribute("data-dock", "shown");
    expect(port.postMessage).toHaveBeenLastCalledWith({
      shell: { dock: expect.objectContaining({ fullscreen: true, visible: true, held: false }) },
    });

    await ask("hold");
    expect(panel).not.toHaveAttribute("data-dock");
    expect(screen.getByRole("button", { name: "Pokaż dok" })).toBeInTheDocument();
  } finally {
    window.matchMedia = media;
  }
});
