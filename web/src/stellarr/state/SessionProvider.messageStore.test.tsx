import "fake-indexeddb/auto";

import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import { createRuntime, type Runtime } from "@/adapters";
import { createMessageStore } from "@/lib/messageStore/messageStore";
import { FakeRealtimeClient, SELF, SERVER } from "@/test/fakes";
import { message } from "@/test/messages";
import { fullApi, type FullApi } from "@/test/runtime";

import { LanguageProvider } from "./LanguageProvider";
import { PortsProvider } from "./PortsProvider";
import { SessionProvider, useSessionContext } from "./SessionProvider";

function Probe() {
  const session = useSessionContext();
  return (
    <>
      <output aria-label="phase">{session.phase}</output>
      <button type="button" onClick={() => void session.logout()}>
        Log out
      </button>
    </>
  );
}

const runtimes: Runtime[] = [];
afterEach(() => {
  for (const runtime of runtimes.splice(0)) runtime.dispose();
});

const phase = () => screen.getByLabelText("phase");

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
  return realtime;
}

/** Puts a window on the device for a server, as a channel read earlier would. */
async function keep(serverId: string): Promise<void> {
  const store = createMessageStore({ userId: SELF.id, serverId });
  await store.keepOnly(["ch-general"]);
  await store.write("ch-general", [message("a", 1)], "cursor", new Set());
}

async function kept(serverId: string): Promise<boolean> {
  const store = createMessageStore({ userId: SELF.id, serverId });
  await store.keepOnly(["ch-general"]);
  return (await store.read("ch-general")) !== undefined;
}

describe("what the device keeps of the channels", () => {
  it("is gone after signing out", async () => {
    await keep(SERVER.id);
    mount(fullApi());
    await waitFor(() => expect(phase()).toHaveTextContent("ready"));

    await userEvent.setup().click(screen.getByRole("button", { name: "Log out" }));
    await waitFor(() => expect(phase()).toHaveTextContent("anonymous"));

    expect(await kept(SERVER.id)).toBe(false);
  });

  it("is gone when the server ends the session", async () => {
    await keep(SERVER.id);
    const realtime = mount(fullApi({ session: vi.fn(async () => null) }));
    await waitFor(() => expect(realtime.getStatus()).toBe("connected"));

    act(() => realtime.drop());
    await waitFor(() => expect(phase()).toHaveTextContent("anonymous"));

    await waitFor(async () => expect(await kept(SERVER.id)).toBe(false));
  });

  it("is gone for the server that shut you out, and kept for the others", async () => {
    await keep(SERVER.id);
    await keep("srv-elsewhere");
    const realtime = mount(fullApi());
    await waitFor(() => expect(phase()).toHaveTextContent("ready"));

    act(() => realtime.push("ServerAccessRevoked", { serverId: SERVER.id, kind: "kicked" }));
    await waitFor(() => expect(phase()).toHaveTextContent("revoked"));

    await waitFor(async () => expect(await kept(SERVER.id)).toBe(false));
    expect(await kept("srv-elsewhere")).toBe(true);
  });
});
