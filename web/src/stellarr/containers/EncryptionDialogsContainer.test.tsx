import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { generateRecoveryCode, normalizeRecoveryCode } from "@/lib/e2ee/recovery";
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
    prepareRecoveryCode: vi.fn(async () => ({ accountKeyId: "key", backup: { version: 2, salt: "c2FsdA==", iterations: 1, verifier: "", iv: "", ciphertext: "" } })),
    changeRecoveryCode: vi.fn(async () => {}),
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

const codeField = () => screen.getByLabelText("Kod odzyskiwania") as HTMLInputElement;
async function activate(user: ReturnType<typeof userEvent.setup>) {
  await user.click(screen.getByRole("checkbox"));
  await user.click(screen.getByRole("button", { name: "Aktywuj kod" }));
}

beforeEach(() => {
  resetStubs();
  security.confirmSecondStep.mockClear();
  security.hasPassword = true;
});

describe("EncryptionDialogsContainer", () => {
  it("saves the generated recovery code and resumes the requested encryption switch", async () => {
    const encryption = port();
    const user = open({ kind: "setup", then: { conversationId: "dm-1", action: "confirm" } }, encryption);

    const generated = codeField().value;
    expect(normalizeRecoveryCode(generated)).toHaveLength(64);
    await activate(user);

    await waitFor(() => expect(stubs.encryption.runSwitch).toHaveBeenCalledWith("dm-1", "confirm"));
    expect(encryption.setUp).toHaveBeenCalledWith(generated);
    expect(stubs.encryption.closeDialog).toHaveBeenCalled();
  });

  it("requires confirmation that the generated code was saved", () => {
    const encryption = port();
    open({ kind: "setup" }, encryption);
    expect(screen.getByRole("button", { name: "Aktywuj kod" })).toBeDisabled();
    expect(encryption.setUp).not.toHaveBeenCalled();
  });

  it("opens the migration dialog after unlocking a legacy backup on a new device", async () => {
    const encryption = port();
    setStubs({ encryption: { status: { ...stubs.encryption.status, migrationRequired: true } } });
    const user = open({ kind: "unlock" }, encryption);
    await user.type(screen.getByLabelText("PIN, 6 cyfr"), "084621");
    await user.click(screen.getByRole("button", { name: "Odblokuj" }));
    expect(encryption.unlock).toHaveBeenCalledWith("084621");
    await waitFor(() => expect(stubs.encryption.openDialog).toHaveBeenCalledWith({ kind: "recoveryCode", mode: "change" }));
  });

  it("says how many attempts are left after an incorrect recovery code, and offers only new keys once the backup is gone", async () => {
    const unlock = vi
      .fn()
      .mockResolvedValueOnce({ kind: "wrong-code", remainingAttempts: 3 })
      .mockResolvedValueOnce({ kind: "backup-deleted" });
    const user = open({ kind: "unlock" }, port({ unlock }));

    await user.type(codeField(), generateRecoveryCode());
    await user.click(screen.getByRole("button", { name: "Odblokuj" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Zostały 3 próby");

    await user.type(codeField(), generateRecoveryCode());
    await user.click(screen.getByRole("button", { name: "Odblokuj" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("kopia klucza została usunięta");
    await user.click(screen.getByRole("button", { name: "Utwórz nowe klucze konta" }));
    expect(stubs.encryption.openDialog).toHaveBeenCalledWith({ kind: "forgot" });
  });

  it("replaces the recovery code behind the password and, when the server asks, a fresh second step", async () => {
    const changeRecoveryCode = vi
      .fn()
      .mockRejectedValueOnce(new ApiError("forbidden", "fresh", 403, { code: "fresh-mfa-required" }))
      .mockResolvedValueOnce(undefined);
    const encryption = port({ changeRecoveryCode });
    const user = open({ kind: "recoveryCode", mode: "change" }, encryption);

    await user.type(screen.getByLabelText("Obecne hasło"), "correct horse");
    await activate(user);

    await waitFor(() => expect(changeRecoveryCode).toHaveBeenCalledTimes(2));
    expect(security.confirmSecondStep).toHaveBeenCalledOnce();
    expect(encryption.prepareRecoveryCode).toHaveBeenCalledOnce();
    expect(changeRecoveryCode.mock.calls[1]?.[1]).toBe("correct horse");
    await waitFor(() => expect(stubs.encryption.closeDialog).toHaveBeenCalled());
  });

  it("goes back to the password when the server refuses it", async () => {
    const changeRecoveryCode = vi.fn().mockRejectedValue(new ApiError("forbidden", "wrong", 403, { code: "password-incorrect" }));
    const user = open({ kind: "recoveryCode", mode: "change" }, port({ changeRecoveryCode }));

    await user.type(screen.getByLabelText("Obecne hasło"), "wrong");
    await activate(user);

    expect(await screen.findByRole("alert")).toHaveTextContent("To nie jest Twoje obecne hasło.");
    expect(screen.getByLabelText("Obecne hasło")).toBeEnabled();
  });

  it("asks an account without a password for no password, and for the fresh proof when the server wants it", async () => {
    security.hasPassword = false;
    const changeRecoveryCode = vi
      .fn()
      .mockRejectedValueOnce(new ApiError("forbidden", "fresh", 403, { code: "fresh-mfa-required" }))
      .mockResolvedValueOnce(undefined);
    const user = open({ kind: "recoveryCode", mode: "change" }, port({ changeRecoveryCode }));

    expect(screen.queryByLabelText("Obecne hasło")).toBeNull();
    await activate(user);

    await waitFor(() => expect(changeRecoveryCode).toHaveBeenCalledTimes(2));
    expect(security.confirmSecondStep).toHaveBeenCalledOnce();
    expect(changeRecoveryCode.mock.calls[1]?.[1]).toBe("");
    // It reads the account for the dialog, and not the sessions the settings list.
    expect(security.opened).toHaveBeenLastCalledWith(true, { withSessions: false });
  });
});
