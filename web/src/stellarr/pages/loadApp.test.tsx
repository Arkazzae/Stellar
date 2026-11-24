import { readFileSync } from "node:fs";
import { join } from "node:path";

import { renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { usePrefetchApp as UsePrefetchApp } from "./loadApp";

const loaded = vi.hoisted(() => vi.fn());
const loadSignedInCode = vi.hoisted(() => vi.fn(async () => ({})));

vi.mock("@/adapters/signedInLoader", () => ({ loadSignedInCode }));

vi.mock("@/containers/AppContainer", () => {
  loaded();
  return { AppContainer: () => null };
});

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  vi.useRealTimers();
});

describe("usePrefetchApp", () => {
  let usePrefetchApp: typeof UsePrefetchApp;

  beforeEach(async () => {
    vi.resetModules();
    loadSignedInCode.mockClear();
    ({ usePrefetchApp } = await import("./loadApp"));
  });
  it("fetches the signed-in app once the browser is idle", async () => {
    let idle: () => void = () => undefined;
    vi.stubGlobal("requestIdleCallback", vi.fn((callback: () => void) => ((idle = callback), 7)));
    vi.stubGlobal("cancelIdleCallback", vi.fn());

    const hook = renderHook(() => usePrefetchApp(true));
    expect(loaded).not.toHaveBeenCalled();
    expect(loadSignedInCode).not.toHaveBeenCalled();

    idle();
    expect(loadSignedInCode).toHaveBeenCalledOnce();
    await vi.waitFor(() => expect(loaded).toHaveBeenCalledOnce());
    hook.unmount();
  });

  it("leaves the connection to the page until the page is usable", () => {
    const request = vi.fn(() => 7);
    vi.stubGlobal("requestIdleCallback", request);
    vi.stubGlobal("cancelIdleCallback", vi.fn());

    const hook = renderHook(({ usable }) => usePrefetchApp(usable), { initialProps: { usable: false } });
    expect(request).not.toHaveBeenCalled();

    hook.rerender({ usable: true });
    expect(request).toHaveBeenCalledOnce();
    hook.unmount();
  });

  it.each([{ saveData: true }, { effectiveType: "slow-2g" }, { effectiveType: "2g" }, { effectiveType: "3g" }])(
    "skips speculative downloads over a constrained connection (%o)",
    (connection) => {
      Object.defineProperty(navigator, "connection", { configurable: true, value: connection });
      const request = vi.fn(() => 7);
      vi.stubGlobal("requestIdleCallback", request);
      try {
        const hook = renderHook(({ signingIn }) => usePrefetchApp(true, signingIn), {
          initialProps: { signingIn: false },
        });
        hook.rerender({ signingIn: true });
        expect(request).not.toHaveBeenCalled();
        expect(loadSignedInCode).not.toHaveBeenCalled();
        hook.unmount();
      } finally {
        Reflect.deleteProperty(navigator, "connection");
      }
    },
  );

  it("cancels speculative idle scheduling when someone starts signing in", () => {
    const request = vi.fn(() => 7);
    const cancel = vi.fn();
    vi.stubGlobal("requestIdleCallback", request);
    vi.stubGlobal("cancelIdleCallback", cancel);

    const hook = renderHook(({ signingIn }) => usePrefetchApp(true, signingIn), {
      initialProps: { signingIn: false },
    });
    expect(loadSignedInCode).not.toHaveBeenCalled();
    hook.rerender({ signingIn: true });
    expect(loadSignedInCode).toHaveBeenCalledOnce();
    expect(request).toHaveBeenCalledOnce();
    expect(cancel).toHaveBeenCalledWith(7);
    hook.unmount();
  });

  it("gives the moment back when the page goes before the browser is idle", () => {
    const cancel = vi.fn();
    vi.stubGlobal("requestIdleCallback", vi.fn(() => 7));
    vi.stubGlobal("cancelIdleCallback", cancel);

    renderHook(() => usePrefetchApp(true)).unmount();

    expect(cancel).toHaveBeenCalledWith(7);
  });

  it("waits a moment instead where the browser has no idle callback", () => {
    vi.useFakeTimers();
    vi.stubGlobal("requestIdleCallback", undefined);
    const setTimer = vi.spyOn(window, "setTimeout");

    const hook = renderHook(() => usePrefetchApp(true));

    expect(setTimer).toHaveBeenCalledWith(expect.any(Function), 1000);
    hook.unmount();
  });
});

describe("which pages fetch the app ahead of need", () => {
  const page = (name: string) => readFileSync(join(__dirname, `${name}.tsx`), "utf8");

  it("are the ones that lead to it, sign-in, registration and invites, and not the ones a mailed link opens", () => {
    for (const name of ["LoginPage", "RegisterPage", "InvitePage"]) {
      expect(page(name), name).toContain("usePrefetchApp(");
    }
    for (const name of ["ResetPasswordPage", "ConfirmEmailPage", "LinkAccountPage", "ForgotPasswordPage"]) {
      expect(page(name), name).not.toContain("usePrefetchApp");
    }
  });
});
