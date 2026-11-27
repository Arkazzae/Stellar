import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type * as PushModule from "@/lib/notifications/push";

const world = vi.hoisted(() => ({
  port: undefined as unknown as {
    getCapabilities: ReturnType<typeof vi.fn>;
    savePushSubscription: ReturnType<typeof vi.fn>;
    deletePushSubscription: ReturnType<typeof vi.fn>;
    releaseDevice: ReturnType<typeof vi.fn>;
  } | undefined,
  account: {
    preferences: { emailNotifications: false, pushNotifications: false },
    setPreferences: vi.fn(),
  },
  user: { id: "u-1" } as { id: string } | null,
}));

const push = vi.hoisted(() => {
  return {
    supported: vi.fn(() => true),
    permission: vi.fn((): NotificationPermission | "unsupported" => "granted"),
    requestPermission: vi.fn(),
    decode: vi.fn(() => new Uint8Array([1, 2])),
    inspect: vi.fn(async () => ({ state: "idle" as const })),
    enroll: vi.fn(),
    disable: vi.fn(),
  };
});

vi.mock("@/state/hooks", () => ({
  usePorts: () => ({ notificationDelivery: world.port }),
  useAccount: () => world.account,
  useSession: () => ({ user: world.user }),
}));

// The error type, its detail and the Brave check are the real ones; only what
// touches the browser's push machinery is replaced.
vi.mock("@/lib/notifications/push", async (importOriginal) => ({
  ...(await importOriginal<typeof PushModule>()),
  pushManagerSupported: push.supported,
  deviceNotificationPermission: push.permission,
  requestDeviceNotificationPermission: push.requestPermission,
  decodeApplicationServerKey: push.decode,
  inspectDevicePush: push.inspect,
  enrollDevicePush: push.enroll,
  disableDevicePush: push.disable,
}));

import { PushEnrollmentError } from "@/lib/notifications/push";

import { useNotificationDelivery } from "./useNotificationDelivery";

const CAPABILITIES = {
  emailAvailable: true,
  emailVerified: true,
  pushAvailable: true,
  applicationServerKey: "AQID",
};

beforeEach(() => {
  world.user = { id: "u-1" };
  world.account = {
    preferences: { emailNotifications: false, pushNotifications: false },
    setPreferences: vi.fn(async () => undefined),
  };
  world.port = {
    getCapabilities: vi.fn(async () => CAPABILITIES),
    savePushSubscription: vi.fn(async () => "row-1"),
    deletePushSubscription: vi.fn(async () => undefined),
    releaseDevice: vi.fn(async () => undefined),
  };
  push.supported.mockReturnValue(true);
  push.permission.mockReturnValue("granted");
  push.requestPermission.mockResolvedValue("granted");
  push.decode.mockReturnValue(new Uint8Array([1, 2]));
  push.inspect.mockResolvedValue({ state: "idle" });
  push.enroll.mockResolvedValue({ id: "row-1", endpoint: "https://push.example/sub" });
  push.disable.mockResolvedValue({ serverRemoved: true, removedLocally: true });
  push.requestPermission.mockClear();
  push.decode.mockClear();
  push.inspect.mockClear();
  push.enroll.mockClear();
  push.disable.mockClear();
});

async function mounted() {
  const view = renderHook(() => useNotificationDelivery(true));
  await waitFor(() => expect(view.result.current?.loading).toBe(false));
  return view;
}

describe("useNotificationDelivery capabilities", () => {
  it("loads what the server can send and keeps device enrollment separate", async () => {
    const { result } = await mounted();

    expect(world.port?.getCapabilities).toHaveBeenCalledOnce();
    expect(result.current).toMatchObject({
      emailAvailable: true,
      emailVerified: true,
      pushAvailable: true,
      pushKeyAvailable: true,
      emailEnabled: false,
      pushEnabled: false,
      device: { status: "idle" },
    });
  });

  it("reports a failed capability read without enabling either switch", async () => {
    world.port!.getCapabilities.mockRejectedValueOnce(new Error("offline"));

    const { result } = await mounted();

    expect(result.current).toMatchObject({
      loadFailed: true,
      emailAvailable: false,
      pushAvailable: false,
    });
  });

  it("saves each global switch on its own", async () => {
    const { result } = await mounted();

    await act(async () => result.current?.onEmailChange(true));
    await act(async () => result.current?.onPushChange(true));

    expect(world.account.setPreferences).toHaveBeenNthCalledWith(1, {
      emailNotifications: true,
    });
    expect(world.account.setPreferences).toHaveBeenNthCalledWith(2, {
      pushNotifications: true,
    });
  });
});

describe("useNotificationDelivery device enrollment", () => {
  it("asks for permission synchronously from the click, before enrollment awaits", async () => {
    let grant: (permission: NotificationPermission) => void = () => undefined;
    push.requestPermission.mockReturnValue(
      new Promise<NotificationPermission>((resolve) => {
        grant = resolve;
      }),
    );
    const order: string[] = [];
    push.requestPermission.mockImplementation(() => {
      order.push("permission");
      return new Promise<NotificationPermission>((resolve) => {
        grant = resolve;
      });
    });
    push.enroll.mockImplementation(async () => {
      order.push("enroll");
      return { id: "row-1", endpoint: "https://push.example/sub" };
    });
    const { result } = await mounted();

    act(() => result.current?.onEnableDevice());

    expect(order).toEqual(["permission"]);
    expect(result.current?.device.pending).toBe(true);

    await act(async () => {
      grant("granted");
      await Promise.resolve();
    });

    await waitFor(() => expect(result.current?.device.status).toBe("enrolled"));
    expect(order).toEqual(["permission", "enroll"]);
    // A device enrollment must not silently turn on the account-wide switch.
    expect(world.account.setPreferences).not.toHaveBeenCalled();
  });

  it("does not subscribe when permission is denied, and changes no account switch", async () => {
    push.requestPermission.mockResolvedValue("denied");
    const { result } = await mounted();

    await act(async () => result.current?.onEnableDevice());

    await waitFor(() => expect(result.current?.device.pending).toBe(false));
    expect(push.enroll).not.toHaveBeenCalled();
    expect(result.current?.device.error).toBe("preference.pushDevicePermissionFailed");
    expect(world.account.setPreferences).not.toHaveBeenCalled();
  });

  it("refuses unsupported browsers and missing server configuration before prompting", async () => {
    push.supported.mockReturnValue(false);
    const unsupported = await mounted();

    act(() => unsupported.result.current?.onEnableDevice());

    expect(push.requestPermission).not.toHaveBeenCalled();
    expect(unsupported.result.current?.device.error).toBe("preference.pushDeviceUnsupported");

    push.supported.mockReturnValue(true);
    world.port!.getCapabilities.mockResolvedValueOnce({
      ...CAPABILITIES,
      applicationServerKey: null,
    });
    const noKey = renderHook(() => useNotificationDelivery(true));
    await waitFor(() => expect(noKey.result.current?.loading).toBe(false));
    act(() => noKey.result.current?.onEnableDevice());

    expect(push.requestPermission).not.toHaveBeenCalled();
    expect(noKey.result.current?.device.error).toBe("preference.pushDeviceUnavailable");
  });

  it("shows the server failure and lets the module roll the subscription back", async () => {
    push.enroll.mockRejectedValueOnce(new PushEnrollmentError("server"));
    const { result } = await mounted();

    await act(async () => result.current?.onEnableDevice());
    await waitFor(() => expect(result.current?.device.pending).toBe(false));

    expect(push.enroll).toHaveBeenCalledOnce();
    expect(result.current?.device.error).toBe("preference.pushDeviceServerFailed");
    expect(result.current?.device.status).toBe("idle");
  });

  it("does not apply a late enrollment after the account changes", async () => {
    let finish: (value: { id: string; endpoint: string }) => void = () => undefined;
    push.enroll.mockReturnValue(
      new Promise((resolve) => {
        finish = resolve;
      }),
    );
    const { result, rerender } = await mounted();

    await act(async () => result.current?.onEnableDevice());
    world.user = { id: "u-2" };
    rerender();
    await act(async () => {
      finish({ id: "row-1", endpoint: "https://push.example/sub" });
      await Promise.resolve();
    });

    expect(result.current?.device.status).toBe("idle");
    expect(result.current?.device.pending).toBe(false);
  });
});

describe("useNotificationDelivery failure messages", () => {
  async function failWith(cause: unknown) {
    push.enroll.mockRejectedValueOnce(cause);
    const view = await mounted();
    await act(async () => view.result.current?.onEnableDevice());
    await waitFor(() => expect(view.result.current?.device.pending).toBe(false));
    return view;
  }

  it("names a missing push service and keeps the browser's words for it", async () => {
    const { result } = await failWith(
      new PushEnrollmentError(
        "push-service",
        new DOMException("Registration failed - push service error", "AbortError"),
      ),
    );

    expect(result.current?.device).toMatchObject({
      error: "preference.pushDeviceServiceFailed",
      errorDetail: "AbortError – Registration failed - push service error",
      status: "idle",
    });

    // The next attempt starts clean and, once it works, leaves nothing behind.
    await act(async () => result.current?.onEnableDevice());
    await waitFor(() => expect(result.current?.device.status).toBe("enrolled"));
    expect(result.current?.device).toMatchObject({ error: null, errorDetail: null });
  });

  it("points Brave at its own push setting", async () => {
    Object.defineProperty(navigator, "brave", { value: { isBrave: () => true }, configurable: true });
    try {
      const { result } = await failWith(
        new PushEnrollmentError(
          "push-service",
          new DOMException("Registration failed - push service error", "AbortError"),
        ),
      );

      expect(result.current?.device.error).toBe("preference.pushDeviceServiceBrave");
    } finally {
      Reflect.deleteProperty(navigator, "brave");
    }
  });

  it.each([
    ["denied", "preference.pushDeviceDenied"],
    ["key-mismatch", "preference.pushDeviceKeyMismatch"],
    ["timeout", "preference.pushDeviceTimeout"],
    ["malformed", "preference.pushDeviceMalformed"],
    ["worker", "preference.pushDeviceWorkerFailed"],
  ] as const)("says what a %s failure means", async (code, message) => {
    const { result } = await failWith(new PushEnrollmentError(code));

    expect(result.current?.device.error).toBe(message);
    // Nothing came from the browser, so there is nothing to quote.
    expect(result.current?.device.errorDetail).toBeNull();
  });

  it("lets a cancelled enrollment go without calling it a failure", async () => {
    const { result } = await failWith(new PushEnrollmentError("cancelled"));

    expect(result.current?.device).toMatchObject({
      pending: false,
      error: null,
      errorDetail: null,
      status: "idle",
    });
  });

  it("keeps an unexpected error generic but quotes it", async () => {
    const { result } = await failWith(new TypeError("boom"));

    expect(result.current?.device).toMatchObject({
      error: "preference.pushDeviceSubscribeFailed",
      errorDetail: "TypeError – boom",
    });
  });

  it("treats a refused permission request as a permission problem, not a push one", async () => {
    push.requestPermission.mockImplementationOnce(() =>
      Promise.reject(new DOMException("The request is not allowed.", "SecurityError")),
    );
    const { result } = await mounted();

    await act(async () => result.current?.onEnableDevice());
    await waitFor(() => expect(result.current?.device.pending).toBe(false));

    expect(push.enroll).not.toHaveBeenCalled();
    expect(result.current?.device).toMatchObject({
      error: "preference.pushDevicePermissionFailed",
      errorDetail: "SecurityError – The request is not allowed.",
    });
  });
});
