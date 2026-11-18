import { act, render } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { ConfirmProvider } from "@/components/primitives/ConfirmDialog";
import type { DmConversation, EncryptionPort, EncryptionStatus } from "@/ports";
import { answerConfirm } from "@/test/confirm";

import { EncryptionProvider, useEncryptionContext, type EncryptionValue } from "./EncryptionProvider";

const SELF = "u-me";
const harness = vi.hoisted(() => ({ port: undefined as unknown as EncryptionPort }));

vi.mock("./PortsProvider", () => ({ usePorts: () => ({ encryption: harness.port }) }));
vi.mock("./SessionProvider", () => ({ useSessionContext: () => ({ user: { id: "u-me" } }) }));

function status(overrides: Partial<EncryptionStatus> = {}): EncryptionStatus {
  return {
    support: "supported",
    accountKey: { keyId: "k-1", publicKey: "AA==" },
    backup: { remainingAttempts: 10 },
    unlocked: true,
    deviceId: "d-1",
    deviceLimit: false,
    revision: 0,
    ...overrides,
  };
}

function port(current: EncryptionStatus): EncryptionPort {
  return {
    getStatus: () => current,
    subscribe: () => () => undefined,
    switchConversation: vi.fn(async () => ({ state: "requested", requestedById: SELF, requestedAt: null, enabledAt: null, currentEpochId: null })),
  } as unknown as EncryptionPort;
}

function conversation(encryption?: DmConversation["encryption"]): DmConversation {
  return {
    id: "dm-1",
    peer: { id: "u-maja", username: "maja", displayName: "Maja", presence: "online" },
    unread: 0,
    updatedAt: "2026-11-18T10:00:00Z",
    ...(encryption === undefined ? {} : { encryption }),
  };
}

function mount() {
  const seen: { value: EncryptionValue | null } = { value: null };
  function Probe() {
    seen.value = useEncryptionContext();
    return null;
  }
  render(
    <ConfirmProvider>
      <EncryptionProvider>
        <Probe />
      </EncryptionProvider>
    </ConfirmProvider>,
  );
  return () => {
    if (seen.value === null) throw new Error("not mounted");
    return seen.value;
  };
}

beforeEach(() => {
  harness.port = port(status());
});

describe("EncryptionProvider", () => {
  it("asks for a PIN first when the account has no key, and remembers the switch it was for", () => {
    harness.port = port(status({ accountKey: null, backup: null, unlocked: false }));
    const value = mount();

    act(() => value().switchOn(conversation()));

    expect(value().dialog).toEqual({ kind: "setup", then: { conversationId: "dm-1", action: "request" } });
    expect(harness.port.switchConversation).not.toHaveBeenCalled();
  });

  it("asks, or confirms the other person's request, when the account has a key", () => {
    const value = mount();
    act(() => value().switchOn(conversation()));
    act(() =>
      value().switchOn(conversation({ state: "requested", requestedById: "u-maja", requestedAt: null, enabledAt: null, currentEpochId: null })),
    );
    expect(vi.mocked(harness.port.switchConversation).mock.calls).toEqual([
      ["dm-1", "request"],
      ["dm-1", "confirm"],
    ]);
  });

  it("withdraws your own request at once, and asks before switching encryption off", async () => {
    const user = userEvent.setup();
    const value = mount();
    act(() =>
      value().switchOff(conversation({ state: "requested", requestedById: SELF, requestedAt: null, enabledAt: null, currentEpochId: null })),
    );
    expect(harness.port.switchConversation).toHaveBeenLastCalledWith("dm-1", "cancel");

    act(() => value().switchOff(conversation({ state: "on", requestedById: null, requestedAt: null, enabledAt: "x", currentEpochId: null })));
    await answerConfirm(user, "Wyłącz");
    expect(harness.port.switchConversation).toHaveBeenLastCalledWith("dm-1", "disable");
  });

  it("does nothing where the browser cannot encrypt", () => {
    harness.port = port(status({ support: "insecure" }));
    const value = mount();
    act(() => value().switchOn(conversation()));
    expect(value().dialog).toBeNull();
    expect(harness.port.switchConversation).not.toHaveBeenCalled();
  });
});
