import { Router } from "@stellarr/router";
import { createMemoryHistory, type MemoryHistory } from "@stellarr/router/testing";
import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { routes } from "@/routes";

import { InstanceSetupProvider, useInstanceSetupContext } from "./InstanceSetupProvider";
import type { SessionPhase } from "./SessionProvider";

const mocks = vi.hoisted(() => {
  const mocks = {
    phase: "loading" as SessionPhase,
    setupRequired: (() => Promise.resolve(true)) as () => Promise<boolean>,
    // One object for the life of the page, as the app's runtime gives.
    ports: { session: { setupRequired: () => mocks.setupRequired() } },
  };
  return mocks;
});

vi.mock("./SessionProvider", () => ({ useSessionContext: () => ({ phase: mocks.phase }) }));
vi.mock("./PortsProvider", () => ({ usePorts: () => mocks.ports }));

let history: MemoryHistory;

function Probe() {
  const { failed, retry, markSetUp } = useInstanceSetupContext();
  return (
    <>
      <output aria-label="failed">{String(failed)}</output>
      <button type="button" onClick={retry}>
        retry
      </button>
      <button type="button" onClick={markSetUp}>
        founded
      </button>
    </>
  );
}

async function open(href: string) {
  history = createMemoryHistory(href);
  const view = render(
    <Router routes={routes} history={history}>
      <InstanceSetupProvider>
        <Probe />
      </InstanceSetupProvider>
    </Router>,
  );
  await act(async () => undefined);
  return view;
}

const go = (pathname: string) => act(() => history.push({ pathname, search: "", hash: "" }));

beforeEach(() => {
  mocks.phase = "anonymous";
  mocks.setupRequired = vi.fn(async () => true);
});

describe("InstanceSetupProvider", () => {
  it("leads every address to /setup while the instance has no owner, asking once per page load", async () => {
    await open("/login");
    expect(history.navigations).toEqual([{ kind: "replace", href: "/setup" }]);

    await go("/invite/abc123");
    await act(async () => undefined);

    expect(history.href).toBe("/setup");
    expect(mocks.setupRequired).toHaveBeenCalledOnce();
  });

  it("stays out of the way once the instance has an owner, and once the setup page founds it", async () => {
    mocks.setupRequired = vi.fn(async () => false);
    const view = await open("/register");
    expect(history.navigations).toEqual([]);

    mocks.setupRequired = vi.fn(async () => true);
    view.unmount();
    await open("/setup");
    await userEvent.click(screen.getByRole("button", { name: "founded" }));
    await go("/login");
    expect(history.href).toBe("/login");
  });

  it("asks nothing of a signed-in visitor, and asks again after a failure only when told to", async () => {
    mocks.phase = "ready";
    const view = await open("/");
    expect(mocks.setupRequired).not.toHaveBeenCalled();
    view.unmount();

    mocks.phase = "anonymous";
    mocks.setupRequired = vi.fn<() => Promise<boolean>>().mockRejectedValueOnce(new Error("offline")).mockResolvedValue(true);
    await open("/login");
    expect(screen.getByRole("status", { name: "failed" })).toHaveTextContent("true");
    expect(history.navigations).toEqual([]);

    await userEvent.click(screen.getByRole("button", { name: "retry" }));
    await act(async () => undefined);
    expect(mocks.setupRequired).toHaveBeenCalledTimes(2);
    expect(history.href).toBe("/setup");
  });
});
