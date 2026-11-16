import { Router } from "@stellarr/router";
import { createMemoryHistory, type MemoryHistory } from "@stellarr/router/testing";
import { renderHook } from "@testing-library/react";
import type { ReactNode } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { PENDING_INVITE_STORAGE_KEY, rememberPendingInvite } from "@/lib/auth/invites";
import { routes } from "@/routes";
import type { SessionPhase } from "@/state/hooks";

import { useRedirectWhenSignedIn } from "./useRedirectWhenSignedIn";

const mocks = vi.hoisted(() => ({ leaveFor: vi.fn() }));

vi.mock("@/lib/auth/leaveFor", () => ({ leaveFor: mocks.leaveFor }));

let history: MemoryHistory;

function atLogin({ children }: { children: ReactNode }) {
  return (
    <Router routes={routes} history={history}>
      {children}
    </Router>
  );
}

beforeEach(() => {
  history = createMemoryHistory("/login");
});

afterEach(() => {
  mocks.leaveFor.mockReset();
  window.sessionStorage.clear();
});

describe("useRedirectWhenSignedIn", () => {
  it.each<SessionPhase>(["loading", "anonymous", "error", "revoked"])(
    "stays where it is while the session is %s",
    (phase) => {
      rememberPendingInvite("abc123");

      renderHook(() => useRedirectWhenSignedIn(phase), { wrapper: atLogin });

      expect(history.navigations).toEqual([]);
      expect(window.sessionStorage.getItem(PENDING_INVITE_STORAGE_KEY)).toBe("abc123");
    },
  );

  it("goes home once the session is ready and no invite is waiting", () => {
    renderHook(() => useRedirectWhenSignedIn("ready"), { wrapper: atLogin });

    expect(history.navigations).toEqual([{ kind: "replace", href: "/" }]);
  });

  it("goes to the invite the visitor came with and forgets it", () => {
    rememberPendingInvite("a/b c");

    renderHook(() => useRedirectWhenSignedIn("ready"), { wrapper: atLogin });

    expect(history.navigations).toEqual([{ kind: "replace", href: "/invite/a%2Fb%20c" }]);
    expect(window.sessionStorage.getItem(PENDING_INVITE_STORAGE_KEY)).toBeNull();
  });

  it("redirects when the session turns ready after sign-in", () => {
    const { rerender } = renderHook(({ phase }) => useRedirectWhenSignedIn(phase), {
      initialProps: { phase: "anonymous" as SessionPhase },
      wrapper: atLogin,
    });
    expect(history.navigations).toEqual([]);

    rerender({ phase: "ready" });

    expect(history.navigations).toEqual([{ kind: "replace", href: "/" }]);
  });

  it("goes back with a page load to a return path outside the app's router", () => {
    renderHook(() => useRedirectWhenSignedIn("ready", "/cockpit/users"), { wrapper: atLogin });

    expect(mocks.leaveFor).toHaveBeenCalledExactlyOnceWith("/cockpit/users");
    expect(history.navigations).toEqual([]);
  });
});
