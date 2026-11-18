import "fake-indexeddb/auto";

import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import { createRuntime, type Runtime } from "@/adapters";
import { cacheKey, set } from "@/lib/chatTranslation/translationCache";
import { FakeRealtimeClient } from "@/test/fakes";
import { fullApi } from "@/test/runtime";

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

/** What a page loaded after this one would find on disk. */
async function storedAfterReload(key: string): Promise<string | undefined> {
  vi.resetModules();
  const reloaded = await import("@/lib/chatTranslation/translationCache");
  return reloaded.get(key);
}

describe("signing out and the translation cache", () => {
  it("leaves no stored translation behind, since it can hold the text of private messages", async () => {
    const key = cacheKey("hasło podam ci głosowo", "en");
    await set(key, "I will tell you the password on a call", "en");
    expect(await storedAfterReload(key)).toBe("I will tell you the password on a call");

    const runtime = createRuntime({ api: fullApi(), realtime: new FakeRealtimeClient() });
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
    await waitFor(() => expect(screen.getByLabelText("phase")).toHaveTextContent("ready"));

    await userEvent.setup().click(screen.getByRole("button", { name: "Log out" }));
    await waitFor(() => expect(screen.getByLabelText("phase")).toHaveTextContent("anonymous"));

    expect(await storedAfterReload(key)).toBeUndefined();
  });
});
