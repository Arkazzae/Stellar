import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  NOTIFICATION_WORKER_PATH,
  NOTIFICATION_WORKER_SCOPE,
  PushEnrollmentError,
  decodeApplicationServerKey,
  disableDevicePush,
  enrollDevicePush,
  forgetRememberedSubscription,
  inspectDevicePush,
  releaseDevicePush,
} from "./push";

const REMEMBERED_KEY = "stellarr.push-subscription";
const KEY = new Uint8Array([1, 2, 3, 4]);

const originalNotification = Object.getOwnPropertyDescriptor(window, "Notification");
const originalPushManager = Object.getOwnPropertyDescriptor(window, "PushManager");
const originalServiceWorker = Object.getOwnPropertyDescriptor(navigator, "serviceWorker");

class FakeSubscription {
  readonly endpoint = "https://push.example/sub-1";
  readonly toJSON = () => ({
    endpoint: this.endpoint,
    expirationTime: null,
    keys: { p256dh: "p256dh-key", auth: "auth-secret" },
  });
  readonly unsubscribe = vi.fn(async () => {
    manager.current = null;
    return true;
  });
}

class FakePushManager {
  current: FakeSubscription | null = null;
  last: FakeSubscription | null = null;
  subscribe = vi.fn(async (options: PushSubscriptionOptionsInit) => {
    this.options = options;
    this.last = new FakeSubscription();
    this.current = this.last;
    return this.current as unknown as PushSubscription;
  });
  getSubscription = vi.fn(async () => this.current as unknown as PushSubscription | null);
  options?: PushSubscriptionOptionsInit;
}

class FakeRegistration {
  readonly active = {
    scriptURL: new URL(NOTIFICATION_WORKER_PATH, window.location.href).href,
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
  };
  readonly waiting = null;
  readonly installing = null;
  readonly pushManager = manager;
  readonly scope = "/";
}

let manager: FakePushManager;

function installPlatform(permission: NotificationPermission = "granted") {
  manager = new FakePushManager();
  const registration = new FakeRegistration();
  const serviceWorker = {
    register: vi.fn(async () => registration),
    getRegistration: vi.fn(async () => registration),
  };

  class FakeNotification {
    static permission = permission;
    static requestPermission = vi.fn(async () => permission);
  }
  class FakePushManagerConstructor {}

  Object.defineProperty(window, "Notification", {
    value: FakeNotification,
    configurable: true,
  });
  Object.defineProperty(window, "PushManager", {
    value: FakePushManagerConstructor,
    configurable: true,
  });
  Object.defineProperty(navigator, "serviceWorker", {
    value: serviceWorker,
    configurable: true,
  });

  return { registration, serviceWorker, manager };
}

function remember(userId: string, endpoint = "https://push.example/sub-1") {
  window.localStorage.setItem(REMEMBERED_KEY, JSON.stringify({ userId, endpoint }));
}

beforeEach(() => {
  window.localStorage.clear();
  forgetRememberedSubscription();
});

afterEach(() => {
  vi.restoreAllMocks();
  if (originalNotification === undefined) Reflect.deleteProperty(window, "Notification");
  else Object.defineProperty(window, "Notification", originalNotification);
  if (originalPushManager === undefined) Reflect.deleteProperty(window, "PushManager");
  else Object.defineProperty(window, "PushManager", originalPushManager);
  if (originalServiceWorker === undefined) Reflect.deleteProperty(navigator, "serviceWorker");
  else Object.defineProperty(navigator, "serviceWorker", originalServiceWorker);
  window.localStorage.clear();
});

describe("inspecting Web Push without enrolling", () => {
  it("reads an existing registration and subscription without registering or prompting", async () => {
    const { registration, serviceWorker, manager: push } = installPlatform();
    push.current = new FakeSubscription();
    remember("u-1");

    await expect(inspectDevicePush("u-1")).resolves.toEqual({
      state: "enrolled",
      endpoint: "https://push.example/sub-1",
    });

    expect(serviceWorker.getRegistration).toHaveBeenCalledWith(NOTIFICATION_WORKER_PATH);
    expect(serviceWorker.register).not.toHaveBeenCalled();
    expect(registration.pushManager.getSubscription).toHaveBeenCalledOnce();
  });

  it("calls a subscription left by another account stale", async () => {
    const { manager: push } = installPlatform();
    push.current = new FakeSubscription();
    remember("u-someone-else");

    await expect(inspectDevicePush("u-1")).resolves.toMatchObject({ state: "stale" });
  });

  it("reports an idle device when no subscription exists", async () => {
    installPlatform();

    await expect(inspectDevicePush("u-1")).resolves.toEqual({ state: "idle" });
  });

  it("reports blocked permission before looking for a worker", async () => {
    const { serviceWorker } = installPlatform("denied");

    await expect(inspectDevicePush("u-1")).resolves.toEqual({ state: "blocked" });
    expect(serviceWorker.getRegistration).not.toHaveBeenCalled();
  });

  it("reports an unsupported browser", async () => {
    Reflect.deleteProperty(window, "PushManager");

    await expect(inspectDevicePush("u-1")).resolves.toEqual({ state: "unsupported" });
  });
});

describe("enrolling this device", () => {
  it("registers the worker and saves the subscription under the current account", async () => {
    const { serviceWorker, manager: push } = installPlatform();
    const save = vi.fn(async () => "push-row-1");
    const deleteEndpoint = vi.fn(async () => undefined);

    const result = await enrollDevicePush({
      userId: "u-1",
      applicationServerKey: KEY,
      save,
      deleteEndpoint,
    });

    expect(serviceWorker.register).toHaveBeenCalledExactlyOnceWith(
      NOTIFICATION_WORKER_PATH,
      { scope: NOTIFICATION_WORKER_SCOPE },
    );
    expect(push.subscribe).toHaveBeenCalledExactlyOnceWith({
      userVisibleOnly: true,
      applicationServerKey: KEY,
    });
    expect(save).toHaveBeenCalledExactlyOnceWith({
      endpoint: "https://push.example/sub-1",
      keys: { p256dh: "p256dh-key", auth: "auth-secret" },
    });
    expect(result).toEqual({ id: "push-row-1", endpoint: "https://push.example/sub-1" });
    expect(window.localStorage.getItem(REMEMBERED_KEY)).toContain("u-1");
    expect(deleteEndpoint).not.toHaveBeenCalled();
  });

  it("removes an older browser subscription before creating this account's", async () => {
    const { manager: push } = installPlatform();
    const old = new FakeSubscription();
    push.current = old;
    remember("u-old", old.endpoint);
    const save = vi.fn(async () => "push-row-new");
    const deleteEndpoint = vi.fn(async () => undefined);

    await enrollDevicePush({
      userId: "u-1",
      applicationServerKey: KEY,
      save,
      deleteEndpoint,
    });

    expect(deleteEndpoint).toHaveBeenCalledExactlyOnceWith(old.endpoint);
    expect(old.unsubscribe).toHaveBeenCalledOnce();
    expect(push.subscribe).toHaveBeenCalledOnce();
    expect(save).toHaveBeenCalledOnce();
  });

  it("rolls a newly created subscription back when the server refuses it", async () => {
    const { manager: push } = installPlatform();
    const save = vi.fn(async () => {
      throw new Error("409");
    });

    await expect(
      enrollDevicePush({
        userId: "u-1",
        applicationServerKey: KEY,
        save,
        deleteEndpoint: vi.fn(async () => undefined),
      }),
    ).rejects.toMatchObject({ code: "server" });

    expect(push.last?.unsubscribe).toHaveBeenCalledOnce();
    expect(window.localStorage.getItem(REMEMBERED_KEY)).toBeNull();
  });

  it("refuses to enroll an unsupported browser", async () => {
    Reflect.deleteProperty(window, "PushManager");

    await expect(
      enrollDevicePush({
        userId: "u-1",
        applicationServerKey: KEY,
        save: vi.fn(async () => "row"),
        deleteEndpoint: vi.fn(async () => undefined),
      }),
    ).rejects.toBeInstanceOf(PushEnrollmentError);
  });

  it("rolls a subscription back when the account changed during the server save", async () => {
    const { manager: push } = installPlatform();
    const deleteEndpoint = vi.fn(async () => undefined);
    let checks = 0;

    await expect(
      enrollDevicePush({
        userId: "u-1",
        applicationServerKey: KEY,
        save: vi.fn(async () => "row"),
        deleteEndpoint,
        // The check before the save passes; the one after it does not.
        isStillWanted: () => {
          checks += 1;
          return checks === 1;
        },
      }),
    ).rejects.toMatchObject({ code: "cancelled" });

    expect(deleteEndpoint).toHaveBeenCalledWith("https://push.example/sub-1");
    expect(push.last?.unsubscribe).toHaveBeenCalledOnce();
    expect(window.localStorage.getItem(REMEMBERED_KEY)).toBeNull();
  });
});

describe("turning this device off", () => {
  it("removes the owned server row and unsubscribes locally", async () => {
    const { manager: push } = installPlatform();
    push.current = new FakeSubscription();
    remember("u-1");
    const deleteEndpoint = vi.fn(async () => undefined);

    await expect(
      disableDevicePush({ userId: "u-1", deleteEndpoint }),
    ).resolves.toEqual({ serverRemoved: true, removedLocally: true });

    expect(deleteEndpoint).toHaveBeenCalledExactlyOnceWith("https://push.example/sub-1");
    expect(push.current).toBeNull();
    expect(window.localStorage.getItem(REMEMBERED_KEY)).toBeNull();
  });

  it("unsubscribes locally even when the server cannot be told", async () => {
    const { manager: push } = installPlatform();
    push.current = new FakeSubscription();
    remember("u-1");
    const deleteEndpoint = vi.fn(async () => {
      throw new Error("offline");
    });

    await expect(
      disableDevicePush({ userId: "u-1", deleteEndpoint }),
    ).resolves.toEqual({ serverRemoved: false, removedLocally: true });

    expect(push.current).toBeNull();
    expect(window.localStorage.getItem(REMEMBERED_KEY)).toBeNull();
  });

  it("does not touch another account's server row", async () => {
    const { manager: push } = installPlatform();
    push.current = new FakeSubscription();
    remember("u-someone-else");
    const deleteEndpoint = vi.fn(async () => undefined);

    await disableDevicePush({ userId: "u-1", deleteEndpoint });

    expect(deleteEndpoint).not.toHaveBeenCalled();
    expect(push.current).toBeNull();
  });
});

describe("releasing a device on sign-out", () => {
  it("asks the server first and always unsubscribes locally", async () => {
    const { manager: push } = installPlatform();
    push.current = new FakeSubscription();
    remember("u-1");
    const deleteEndpoint = vi.fn(async () => {
      throw new Error("session already gone");
    });

    await expect(releaseDevicePush({ deleteEndpoint })).resolves.toBeUndefined();

    expect(deleteEndpoint).toHaveBeenCalledOnce();
    expect(push.current).toBeNull();
    expect(window.localStorage.getItem(REMEMBERED_KEY)).toBeNull();
  });

  it("is a quiet no-op when no worker was ever registered", async () => {
    const { serviceWorker } = installPlatform();
    (serviceWorker.getRegistration as ReturnType<typeof vi.fn>).mockResolvedValueOnce(undefined);

    await expect(releaseDevicePush({ deleteEndpoint: vi.fn() })).resolves.toBeUndefined();
    expect(window.localStorage.getItem(REMEMBERED_KEY)).toBeNull();
  });
});

describe("the application server key", () => {
  it("decodes base64url before a permission prompt can happen", () => {
    expect([...decodeApplicationServerKey("AQIDBA")]).toEqual([1, 2, 3, 4]);
  });

  it("rejects an unusable key without enrolling", () => {
    expect(() => decodeApplicationServerKey("")).toThrow(PushEnrollmentError);
  });
});
