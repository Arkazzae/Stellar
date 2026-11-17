import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { createRuntime, type Runtime } from "@/adapters";
import { NOTIFICATION_WORKER_PATH } from "@/lib/notifications/push";
import { BOOTSTRAP, FakeRealtimeClient, SERVER } from "@/test/fakes";
import { fullApi, type FullApi } from "@/test/runtime";

import { LanguageProvider } from "./LanguageProvider";
import { PortsProvider } from "./PortsProvider";
import { SessionProvider, useSessionContext } from "./SessionProvider";

const ENDPOINT = "https://push.example/device-1";
const originalNotification = Object.getOwnPropertyDescriptor(window, "Notification");
const originalPushManager = Object.getOwnPropertyDescriptor(window, "PushManager");
const originalServiceWorker = Object.getOwnPropertyDescriptor(navigator, "serviceWorker");

const subscription = {
  endpoint: ENDPOINT,
  unsubscribe: vi.fn(async () => {
    currentSubscription = null;
    return true;
  }),
};
let currentSubscription: typeof subscription | null = subscription;
const pushManager = {
  getSubscription: vi.fn(async () => currentSubscription as unknown as PushSubscription | null),
};
const serviceWorker = {
  getRegistration: vi.fn(async () => ({
    active: { scriptURL: new URL(NOTIFICATION_WORKER_PATH, window.location.href).href },
    waiting: null,
    installing: null,
    pushManager,
  })),
  register: vi.fn(),
};

function Probe() {
  const session = useSessionContext();
  return (
    <div>
      <output aria-label="phase">{session.phase}</output>
      <button type="button" onClick={() => void session.logout().catch(() => undefined)}>
        Log out
      </button>
    </div>
  );
}

const runtimes: Runtime[] = [];

function mount(api: FullApi) {
  const realtime = new FakeRealtimeClient();
  const runtime = createRuntime({ api, realtime });
  runtimes.push(runtime);
  render(
    <LanguageProvider initialLocale="pl">
      <PortsProvider runtime={runtime}>
        <SessionProvider>
          <Probe />
        </SessionProvider>
      </PortsProvider>
    </LanguageProvider>,
  );
  return { api, realtime };
}

beforeEach(() => {
  window.localStorage.setItem(
    "stellarr.push-subscription",
    JSON.stringify({ userId: "u-maja", endpoint: ENDPOINT }),
  );
  currentSubscription = subscription;
  subscription.unsubscribe.mockClear();
  pushManager.getSubscription.mockClear();
  serviceWorker.getRegistration.mockClear();
  class FakeNotification {
    static permission = "granted" as const;
    static requestPermission = vi.fn(async () => "granted" as const);
  }
  class FakePushManager {}
  Object.defineProperty(window, "Notification", { value: FakeNotification, configurable: true });
  Object.defineProperty(window, "PushManager", { value: FakePushManager, configurable: true });
  Object.defineProperty(navigator, "serviceWorker", { value: serviceWorker, configurable: true });
});

afterEach(() => {
  for (const runtime of runtimes.splice(0)) runtime.dispose();
  window.localStorage.clear();
  if (originalNotification === undefined) Reflect.deleteProperty(window, "Notification");
  else Object.defineProperty(window, "Notification", originalNotification);
  if (originalPushManager === undefined) Reflect.deleteProperty(window, "PushManager");
  else Object.defineProperty(window, "PushManager", originalPushManager);
  if (originalServiceWorker === undefined) Reflect.deleteProperty(navigator, "serviceWorker");
  else Object.defineProperty(navigator, "serviceWorker", originalServiceWorker);
});

describe("session-ending push cleanup", () => {
  it("removes the device subscription before asking the server to log out", async () => {
    const user = userEvent.setup();
    const api = fullApi();
    mount(api);
    await waitFor(() => expect(screen.getByLabelText("phase")).toHaveTextContent("ready"));

    await user.click(screen.getByRole("button", { name: "Log out" }));

    await waitFor(() => expect(screen.getByLabelText("phase")).toHaveTextContent("anonymous"));
    expect(api.deletePushSubscription).toHaveBeenCalledExactlyOnceWith(ENDPOINT);
    expect(subscription.unsubscribe).toHaveBeenCalledOnce();
    expect(vi.mocked(api.deletePushSubscription).mock.invocationCallOrder[0]).toBeLessThan(
      vi.mocked(api.logout).mock.invocationCallOrder[0] ?? 0,
    );
  });

  it("frees the device when the server revokes the open server", async () => {
    const api = fullApi();
    const { realtime } = mount(api);
    await waitFor(() => expect(screen.getByLabelText("phase")).toHaveTextContent("ready"));

    act(() => realtime.push("ServerAccessRevoked", { serverId: SERVER.id, kind: "kicked" }));

    await waitFor(() => expect(screen.getByLabelText("phase")).toHaveTextContent("revoked"));
    await waitFor(() => expect(subscription.unsubscribe).toHaveBeenCalledOnce());
  });
});
