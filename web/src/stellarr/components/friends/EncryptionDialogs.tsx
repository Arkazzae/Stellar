import { useId, useRef, useState, type FormEvent, type ReactNode } from "react";

import { Button } from "@stellarr/ui";
import { Avatar } from "@/components/primitives/Avatar";
import type { StickerName } from "@/components/primitives/MascotArt";
import { Modal } from "@/components/primitives/Modal";
import { DialogHeader } from "@/components/settings/DialogHeader";
import type { PersonVM } from "@/components/view-models";
import { useLanguage } from "@/state/LanguageProvider";

import { PinInput, useSettledFocus } from "./PinInput";
import { MESSAGES_STICKERS } from "./stickers";

const PIN_LENGTH = 6;

/** The current password, focused once the dialog has settled. */
function PasswordField({
  id,
  value,
  disabled,
  invalid,
  describedBy,
  onChange,
}: {
  id: string;
  value: string;
  disabled: boolean;
  invalid: boolean;
  describedBy?: string;
  onChange(value: string): void;
}) {
  const field = useRef<HTMLInputElement>(null);
  useSettledFocus(field);
  return (
    <input
      ref={field}
      id={id}
      className="set-input"
      type="password"
      autoComplete="current-password"
      value={value}
      disabled={disabled}
      aria-invalid={invalid || undefined}
      aria-describedby={describedBy}
      onChange={(event) => onChange(event.target.value)}
    />
  );
}

/** The short-dialog frame every encryption dialog shares, as a form. */
function EncryptionFrame({
  open,
  title,
  detail,
  sticker,
  onClose,
  closeDisabled = false,
  onSubmit,
  children,
  foot,
}: {
  open: boolean;
  title: string;
  detail: string;
  /** The mascot above the title, on the dialogs that mark a first step. */
  sticker?: StickerName;
  onClose(): void;
  closeDisabled?: boolean;
  onSubmit(): void;
  children: ReactNode;
  foot: ReactNode;
}) {
  const titleId = useId();
  return (
    <Modal
      open={open}
      onClose={() => {
        if (!closeDisabled) onClose();
      }}
      labelledBy={titleId}
      className="act-modal enc-dialog"
      exitMs={200}
    >
      <form
        className="am-card"
        noValidate
        onSubmit={(event: FormEvent) => {
          event.preventDefault();
          onSubmit();
        }}
      >
        <DialogHeader
          titleId={titleId}
          title={title}
          detail={detail}
          sticker={sticker}
          onClose={onClose}
          closeDisabled={closeDisabled}
        />
        <div className="dlg-body enc-body">{children}</div>
        <div className="dlg-foot">{foot}</div>
      </form>
    </Modal>
  );
}

/** Clears a dialog's fields each time it opens, in the render that opens it. */
function useOpening(open: boolean, reset: () => void) {
  const [wasOpen, setWasOpen] = useState(open);
  if (wasOpen !== open) {
    setWasOpen(open);
    if (open) reset();
  }
}

export interface PinSetupDialogProps {
  open: boolean;
  pending: boolean;
  error: string | null;
  onSubmit(pin: string): void;
  onClose(): void;
}

/** The first PIN: typed, then typed again. */
export function PinSetupDialog({ open, pending, error, onSubmit, onClose }: PinSetupDialogProps) {
  const { t } = useLanguage();
  const errorId = useId();
  const [step, setStep] = useState<1 | 2>(1);
  const [first, setFirst] = useState("");
  const [second, setSecond] = useState("");
  const [mismatch, setMismatch] = useState(false);
  useOpening(open, () => {
    setStep(1);
    setFirst("");
    setSecond("");
    setMismatch(false);
  });

  const value = step === 1 ? first : second;
  const complete = value.length === PIN_LENGTH;
  const shown = mismatch ? t("e2ee.setup.mismatch") : error;

  const submit = () => {
    if (!complete || pending) return;
    if (step === 1) {
      setStep(2);
      setSecond("");
      return;
    }
    if (second !== first) {
      setMismatch(true);
      setStep(1);
      setFirst("");
      setSecond("");
      return;
    }
    onSubmit(first);
  };

  return (
    <EncryptionFrame
      open={open}
      title={t("e2ee.setup.title")}
      detail={t("e2ee.setup.detail")}
      sticker={MESSAGES_STICKERS.pinSetup}
      onClose={onClose}
      closeDisabled={pending}
      onSubmit={submit}
      foot={
        <>
          <Button type="button" variant="secondary" disabled={pending} onClick={onClose}>
            {t("common.cancel")}
          </Button>
          <Button type="submit" variant="primary" disabled={!complete || pending}>
            {pending ? t("e2ee.setup.working") : step === 1 ? t("e2ee.setup.next") : t("e2ee.setup.done")}
          </Button>
        </>
      }
    >
      <span className="enc-step">{t("e2ee.setup.step", { step })}</span>
      {step === 2 ? <p className="enc-lead">{t("e2ee.setup.repeat")}</p> : null}
      <PinInput
        key={step}
        value={value}
        onChange={(next) => {
          setMismatch(false);
          if (step === 1) setFirst(next);
          else setSecond(next);
        }}
        label={t("e2ee.pin.label")}
        invalid={shown !== null}
        disabled={pending}
        {...(shown === null ? {} : { describedBy: errorId })}
      />
      {shown === null ? null : (
        <p className="enc-err" id={errorId} role="alert">
          {shown}
        </p>
      )}
      <p className="enc-note">{t("e2ee.setup.note")}</p>
    </EncryptionFrame>
  );
}

export interface UnlockDialogProps {
  open: boolean;
  pending: boolean;
  error: string | null;
  /** The backup cannot be opened any more (gone, or not this account's key): only a reset helps. */
  dead: boolean;
  onSubmit(pin: string): void;
  onForgot(): void;
  onClose(): void;
}

/** The PIN, on a browser that does not hold the account key yet. */
export function UnlockDialog({ open, pending, error, dead, onSubmit, onForgot, onClose }: UnlockDialogProps) {
  const { t } = useLanguage();
  const errorId = useId();
  const [pin, setPin] = useState("");
  useOpening(open, () => setPin(""));
  // A refusal clears what was typed, so the next attempt starts empty.
  const [lastError, setLastError] = useState(error);
  if (lastError !== error) {
    setLastError(error);
    if (error !== null) setPin("");
  }

  return (
    <EncryptionFrame
      open={open}
      title={t("e2ee.unlock.title")}
      detail={t("e2ee.unlock.detail")}
      onClose={onClose}
      closeDisabled={pending}
      onSubmit={() => {
        if (dead) onForgot();
        else if (pin.length === PIN_LENGTH && !pending) onSubmit(pin);
      }}
      foot={
        <>
          <Button type="button" variant="secondary" disabled={pending} onClick={onClose}>
            {t("common.cancel")}
          </Button>
          {dead ? (
            <Button type="submit" variant="danger-solid">
              {t("e2ee.forgot.submit")}
            </Button>
          ) : (
            <Button type="submit" variant="primary" disabled={pin.length !== PIN_LENGTH || pending}>
              {pending ? t("e2ee.unlock.working") : t("e2ee.unlock.submit")}
            </Button>
          )}
        </>
      }
    >
      {dead ? null : (
        <PinInput
          value={pin}
          onChange={setPin}
          label={t("e2ee.pin.label")}
          invalid={error !== null}
          disabled={pending}
          {...(error === null ? {} : { describedBy: errorId })}
        />
      )}
      {error === null ? null : (
        <p className="enc-err" id={errorId} role="alert">
          {error}
        </p>
      )}
      {dead ? null : (
        <button type="button" className="enc-link" disabled={pending} onClick={onForgot}>
          {t("e2ee.unlock.forgot")}
        </button>
      )}
    </EncryptionFrame>
  );
}

export interface ForgotPinDialogProps {
  open: boolean;
  onContinue(): void;
  onClose(): void;
}

/** What a reset costs, said before it starts. */
export function ForgotPinDialog({ open, onContinue, onClose }: ForgotPinDialogProps) {
  const { t } = useLanguage();
  return (
    <EncryptionFrame
      open={open}
      title={t("e2ee.forgot.title")}
      detail={t("e2ee.forgot.detail")}
      onClose={onClose}
      onSubmit={onContinue}
      foot={
        <>
          <Button type="button" variant="secondary" onClick={onClose}>
            {t("common.cancel")}
          </Button>
          <Button type="submit" variant="danger-solid">
            {t("e2ee.forgot.submit")}
          </Button>
        </>
      }
    >
      <p className="enc-note">{t("e2ee.forgot.note")}</p>
    </EncryptionFrame>
  );
}

export interface NewPinDialogProps {
  open: boolean;
  /** A new PIN for the key this browser holds, or new keys altogether. */
  mode: "change" | "reset";
  pending: boolean;
  /** The current password was refused: the dialog goes back to it. */
  passwordError: string | null;
  error: string | null;
  onSubmit(input: { password: string; pin: string }): void;
  onClose(): void;
}

type NewPinStep = "password" | "pin" | "repeat";

/**
 * The current password first, as for a password change, then the new PIN
 * twice. The second step of signing in, when the server asks for it, is the
 * security dialog's, raised by whoever sends this.
 */
export function NewPinDialog({ open, mode, pending, passwordError, error, onSubmit, onClose }: NewPinDialogProps) {
  const { t } = useLanguage();
  const errorId = useId();
  const passwordId = useId();
  const [step, setStep] = useState<NewPinStep>("password");
  const [password, setPassword] = useState("");
  const [pin, setPin] = useState("");
  const [again, setAgain] = useState("");
  const [mismatch, setMismatch] = useState(false);
  useOpening(open, () => {
    setStep("password");
    setPassword("");
    setPin("");
    setAgain("");
    setMismatch(false);
  });
  const [lastPasswordError, setLastPasswordError] = useState(passwordError);
  if (lastPasswordError !== passwordError) {
    setLastPasswordError(passwordError);
    if (passwordError !== null) {
      setStep("password");
      setPassword("");
    }
  }

  const shown = step === "password" ? passwordError : mismatch ? t("e2ee.setup.mismatch") : error;
  const ready =
    step === "password" ? password.length > 0 : (step === "pin" ? pin : again).length === PIN_LENGTH;

  const submit = () => {
    if (!ready || pending) return;
    if (step === "password") {
      setStep("pin");
      return;
    }
    if (step === "pin") {
      setStep("repeat");
      setAgain("");
      return;
    }
    if (again !== pin) {
      setMismatch(true);
      setStep("pin");
      setPin("");
      setAgain("");
      return;
    }
    onSubmit({ password, pin });
  };

  return (
    <EncryptionFrame
      open={open}
      title={t(mode === "reset" ? "e2ee.reset.title" : "e2ee.newPin.title")}
      detail={t(step === "password" ? "e2ee.password.detail" : mode === "reset" ? "e2ee.reset.detail" : "e2ee.newPin.detail")}
      onClose={onClose}
      closeDisabled={pending}
      onSubmit={submit}
      foot={
        <>
          <Button type="button" variant="secondary" disabled={pending} onClick={onClose}>
            {t("common.cancel")}
          </Button>
          <Button type="submit" variant={mode === "reset" && step === "repeat" ? "danger-solid" : "primary"} disabled={!ready || pending}>
            {pending
              ? t("e2ee.setup.working")
              : step === "repeat"
                ? t(mode === "reset" ? "e2ee.forgot.submit" : "e2ee.setup.done")
                : t("e2ee.setup.next")}
          </Button>
        </>
      }
    >
      {step === "password" ? (
        <div className="enc-field">
          <label className="fld-lab" htmlFor={passwordId}>
            {t("e2ee.password.label")}
          </label>
          <PasswordField
            id={passwordId}
            value={password}
            disabled={pending}
            invalid={passwordError !== null}
            {...(passwordError === null ? {} : { describedBy: errorId })}
            onChange={setPassword}
          />
        </div>
      ) : (
        <>
          <span className="enc-step">{t("e2ee.setup.step", { step: step === "pin" ? 1 : 2 })}</span>
          {step === "repeat" ? <p className="enc-lead">{t("e2ee.setup.repeat")}</p> : null}
          <PinInput
            key={step}
            value={step === "pin" ? pin : again}
            onChange={(next) => {
              setMismatch(false);
              if (step === "pin") setPin(next);
              else setAgain(next);
            }}
            label={t("e2ee.pin.label")}
            invalid={shown !== null}
            disabled={pending}
            {...(shown === null ? {} : { describedBy: errorId })}
          />
        </>
      )}
      {shown === null ? null : (
        <p className="enc-err" id={errorId} role="alert">
          {shown}
        </p>
      )}
      {step !== "password" ? <p className="enc-note">{t(mode === "reset" ? "e2ee.forgot.note" : "e2ee.setup.note")}</p> : null}
    </EncryptionFrame>
  );
}

export interface SafetyCodeDialogProps {
  open: boolean;
  self: PersonVM;
  peer: PersonVM;
  /** 12 groups of 5 digits; null while they are being worked out. */
  groups: string[] | null;
  error: string | null;
  /** The person said the numbers differ: the dialog says what that means. */
  mismatch: boolean;
  onSame(): void;
  onDiffer(): void;
  onClose(): void;
}

/** The numbers two people compare to know their keys were not swapped. */
export function SafetyCodeDialog({ open, self, peer, groups, error, mismatch, onSame, onDiffer, onClose }: SafetyCodeDialogProps) {
  const { t } = useLanguage();
  return (
    <EncryptionFrame
      open={open}
      title={t("e2ee.safety.title")}
      detail={t("e2ee.safety.detail")}
      sticker={MESSAGES_STICKERS.safetyCode}
      onClose={onClose}
      onSubmit={mismatch ? onClose : onSame}
      foot={
        mismatch ? (
          <Button type="submit" variant="secondary">
            {t("e2ee.safety.close")}
          </Button>
        ) : (
          <>
            <Button type="button" variant="secondary" disabled={groups === null} onClick={onDiffer}>
              {t("e2ee.safety.differ")}
            </Button>
            <Button type="submit" variant="primary" disabled={groups === null}>
              {t("e2ee.safety.same")}
            </Button>
          </>
        )
      }
    >
      <div className="enc-pair">
        <Avatar id={self.id} displayName={self.displayName} initials={self.initials} avatarUrl={self.avatarUrl} size="xs" />
        <span>{t("e2ee.safety.pair", { name: peer.displayName })}</span>
        <Avatar id={peer.id} displayName={peer.displayName} initials={peer.initials} avatarUrl={peer.avatarUrl} size="xs" />
      </div>
      {error !== null ? (
        <p className="enc-err" role="alert">
          {error}
        </p>
      ) : groups === null ? (
        <p className="enc-note" role="status">
          {t("e2ee.safety.loading")}
        </p>
      ) : (
        <div className="enc-code" aria-label={t("e2ee.safety.title")}>
          {groups.map((group, index) => (
            <span key={index}>{group}</span>
          ))}
        </div>
      )}
      {mismatch ? (
        <p className="enc-err" role="alert">
          {t("e2ee.safety.mismatch")}
        </p>
      ) : null}
    </EncryptionFrame>
  );
}
