import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { ApiError } from "@/lib/api/errors";
import type { EncryptionPort } from "@/ports";
import type { EncryptionDialog } from "@/state/hooks";
import { resetStubs, setStubs, stubs } from "@/test/hookStubs";

import { EncryptionDialogsContainer } from "./EncryptionDialogsContainer";

vi.mock("@/state/hooks", async () => (await import("@/test/hookStubs")).hooksModule);

const security = vi.hoisted(() => ({ confirmSecondStep: vi.fn(async () => true), hasPassword: true, opened: vi.fn() }));
vi.mock("./useAccountSecurity", () => ({
  useAccountSecurity: (...args: unknown[]) => {
    security.opened(...args);
    return {
      confirmSecondStep: security.confirmSecondStep,
      sections: { security: { hasPassword: security.hasPassword } },
      dialog: { step: null, pending: false, error: null, emailRetryAfter: null, onClose: vi.fn() },
    };
  },
}));

function port(overrides: Partial<EncryptionPort> = {}): EncryptionPort {
  return {
    setUp: vi.fn(async () => {}),
    unlock: vi.fn(async () => ({ kind: "unlocked" as const })),
    prepareNewPin: vi.fn(async (pin: string) => ({ backup: { salt: pin, iterations: 1, verifier: "", iv: "", ciphertext: "" } })),
    changePin: vi.fn(async () => {}),
    prepareReset: vi.fn(),
    reset: vi.fn(),
    safetyCode: vi.fn(async () => null),
    ...overrides,
  } as unknown as EncryptionPort;
}

function open(dialog: EncryptionDialog, encryption: EncryptionPort) {
  setStubs({
    ports: { ...stubs.ports, encryption },
    encryption: { dialog },
  });
  render(<EncryptionDialogsContainer />);
  return userEvent.setup();
}

const pinField = () => screen.getByLabelText("PIN, 6 cyfr");

beforeEach(() => {
  resetStubs();
  security.confirmSecondStep.mockClear();
  security.hasPassword = true;
});

describe("EncryptionDialogsContainer", () => {
  it("sets the first PIN, typed twice, then moves the switch it was raised for", async () => {
    const encryption = port();
    const user = open({ kind: "setup", then: { conversationId: "dm-1", action: "confirm" } }, encryption);

    await user.type(pinField(), "135790");
    await user.click(screen.getByRole("button", { name: "Dalej" }));
    await user.type(pinField(), "135790");
    await user.click(screen.getByRole("button", { name: "Ustaw PIN" }));

    await waitFor(() => expect(stubs.encryption.runSwitch).toHaveBeenCalledWith("dm-1", "confirm"));
    expect(encryption.setUp).toHaveBeenCalledWith("135790");
    expect(stubs.encryption.closeDialog).toHaveBeenCalled();
  });

  it("starts the PIN again when the second one differs", async () => {
    const encryption = port();
    const user = open({ kind: "setup" }, encryption);

    await user.type(pinField(), "135790");
    await user.click(screen.getByRole("button", { name: "Dalej" }));
    await user.type(pinField(), "999999");
    await user.click(screen.getByRole("button", { name: "Ustaw PIN" }));

    expect(screen.getByRole("alert")).toHaveTextContent("PIN-y się różnią");
    expect(encryption.setUp).not.toHaveBeenCalled();
  });

  it("says how many attempts are left after a wrong PIN, and offers only new keys once the backup is gone", async () => {
    const unlock = vi
      .fn()
      .mockResolvedValueOnce({ kind: "wrong-pin", remainingAttempts: 3 })
      .mockResolvedValueOnce({ kind: "backup-deleted" });
    const user = open({ kind: "unlock" }, port({ unlock }));

    await user.type(pinField(), "000000");
    await user.click(screen.getByRole("button", { name: "Odczytaj" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Zostały 3 próby");

    await user.type(pinField(), "000001");
    await user.click(screen.getByRole("button", { name: "Odczytaj" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("kopia kluczy została usunięta");
    await user.click(screen.getByRole("button", { name: "Ustaw nowy PIN" }));
    expect(stubs.encryption.openDialog).toHaveBeenCalledWith({ kind: "forgot" });
  });

  it("changes the PIN behind the password and, when the server asks, a fresh second step", async () => {
    const changePin = vi
      .fn()
      .mockRejectedValueOnce(new ApiError("forbidden", "fresh", 403, { code: "fresh-mfa-required" }))
      .mockResolvedValueOnce(undefined);
    const encryption = port({ changePin });
    const user = open({ kind: "newPin", mode: "change" }, encryption);

    await user.type(screen.getByLabelText("Obecne hasło"), "correct horse");
    await user.click(screen.getByRole("button", { name: "Dalej" }));
    await user.type(pinField(), "864200");
    await user.click(screen.getByRole("button", { name: "Dalej" }));
    await user.type(pinField(), "864200");
    await user.click(screen.getByRole("button", { name: "Ustaw PIN" }));

    await waitFor(() => expect(changePin).toHaveBeenCalledTimes(2));
    expect(security.confirmSecondStep).toHaveBeenCalledOnce();
    expect(encryption.prepareNewPin).toHaveBeenCalledOnce();
    expect(changePin.mock.calls[1]?.[1]).toBe("correct horse");
    await waitFor(() => expect(stubs.encryption.closeDialog).toHaveBeenCalled());
  });

  it("goes back to the password when the server refuses it", async () => {
    const changePin = vi.fn().mockRejectedValue(new ApiError("forbidden", "wrong", 403, { code: "password-incorrect" }));
    const user = open({ kind: "newPin", mode: "change" }, port({ changePin }));

    await user.type(screen.getByLabelText("Obecne hasło"), "wrong");
    await user.click(screen.getByRole("button", { name: "Dalej" }));
    await user.type(pinField(), "864200");
    await user.click(screen.getByRole("button", { name: "Dalej" }));
    await user.type(pinField(), "864200");
    await user.click(screen.getByRole("button", { name: "Ustaw PIN" }));

    expect(await screen.findByRole("alert")).toHaveTextContent("To nie jest Twoje obecne hasło.");
    expect(screen.getByLabelText("Obecne hasło")).toHaveValue("");
  });

  it("asks an account without a password for no password, and for the fresh proof when the server wants it", async () => {
    security.hasPassword = false;
    const changePin = vi
      .fn()
      .mockRejectedValueOnce(new ApiError("forbidden", "fresh", 403, { code: "fresh-mfa-required" }))
      .mockResolvedValueOnce(undefined);
    const user = open({ kind: "newPin", mode: "change" }, port({ changePin }));

    expect(screen.queryByLabelText("Obecne hasło")).toBeNull();
    await user.type(pinField(), "864200");
    await user.click(screen.getByRole("button", { name: "Dalej" }));
    await user.type(pinField(), "864200");
    await user.click(screen.getByRole("button", { name: "Ustaw PIN" }));

    await waitFor(() => expect(changePin).toHaveBeenCalledTimes(2));
    expect(security.confirmSecondStep).toHaveBeenCalledOnce();
    expect(changePin.mock.calls[1]?.[1]).toBe("");
    // It reads the account for the dialog, and not the sessions the settings list.
    expect(security.opened).toHaveBeenLastCalledWith(true, { withSessions: false });
  });
});
