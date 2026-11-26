import { useId, useRef, useState, type FormEvent, type ReactNode } from "react";

import { Button } from "@stellarr/ui";
import { Avatar } from "@/components/primitives/Avatar";
import type { StickerName } from "@/components/primitives/MascotArt";
import { Modal } from "@/components/primitives/Modal";
import { DialogHeader } from "@/components/settings/DialogHeader";
import type { PersonVM } from "@/components/view-models";
import { useLanguage } from "@/state/LanguageProvider";

import { generateRecoveryCode, normalizeRecoveryCode } from "@/lib/e2ee/recovery";
import { PinInput, useSettledFocus } from "./PinInput";
import { MESSAGES_STICKERS } from "./stickers";


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
  /**
   * The mascot above the title, on the dialogs that mark a first step. Such a
   * dialog reads down the middle, header and body alike.
   */
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
        <div className={sticker === undefined ? "dlg-body enc-body" : "dlg-body enc-body centered"}>{children}</div>
        <div className="dlg-foot">{foot}</div>
      </form>
    </Modal>
  );
}

/** Mounted for one issuance only. A failed request keeps the same code on screen. */
export function RecoveryCodeDialog({ mode, migration, pending, error, passwordError, withPassword, onSubmit, onClose }: {
  mode: "setup" | "change" | "reset";
  migration: boolean;
  pending: boolean;
  error: string | null;
  passwordError: string | null;
  withPassword: boolean;
  onSubmit(code: string, password: string): void;
  onClose(): void;
}) {
  const { t } = useLanguage();
  const [code] = useState(generateRecoveryCode);
  const [saved, setSaved] = useState(false);
  const [password, setPassword] = useState("");
  const ready = saved && !pending && (!withPassword || password.length > 0);
  const id = useId();
  return (
    <EncryptionFrame open title={t(mode === "reset" ? "e2ee.reset.title" : "e2ee.setup.title")}
      detail={t(migration ? "e2ee.migrate.detail" : mode === "reset" ? "e2ee.forgot.note" : "e2ee.setup.detail")}
      onClose={onClose} closeDisabled={pending} onSubmit={() => { if (ready) onSubmit(code, password); }}
      foot={<>
        <Button type="button" variant="secondary" disabled={pending} onClick={onClose}>{t("common.cancel")}</Button>
        <Button type="submit" variant={mode === "reset" ? "danger-solid" : "primary"} disabled={!ready}>
          {t(pending ? "e2ee.setup.working" : "e2ee.setup.done")}
        </Button>
      </>}>
      <label className="enc-field">
        <span className="fld-lab">{t("e2ee.recovery.label")}</span>
        <textarea className="set-input" rows={3} readOnly spellCheck={false} value={code}
          onFocus={(event) => event.currentTarget.select()} />
      </label>
      <label><input type="checkbox" disabled={pending} checked={saved} onChange={(e) => setSaved(e.target.checked)} /> {t("e2ee.recovery.saved")}</label>
      {withPassword && <div className="enc-field">
        <label className="fld-lab" htmlFor={id}>{t("e2ee.password.label")}</label>
        <PasswordField id={id} value={password} invalid={passwordError !== null} disabled={pending} onChange={setPassword} />
      </div>}
      {(passwordError || error) && <p className="enc-err" role="alert">{passwordError ?? error}</p>}
    </EncryptionFrame>
  );
}

export interface UnlockDialogProps {
  legacy: boolean;
  open: boolean;
  pending: boolean;
  error: string | null;
  /** The backup cannot be opened any more (gone, or not this account's key): only a reset helps. */
  dead: boolean;
  onSubmit(input: string): void;
  onForgot(): void;
  onClose(): void;
}

/** A new browser opens either the current backup or a legacy backup for migration. */
export function UnlockDialog({ open, legacy, pending, error, dead, onSubmit, onForgot, onClose }: UnlockDialogProps) {
  const { t } = useLanguage();
  const errorId = useId();
  const [input, setInput] = useState("");
  // A refusal clears what was typed, so the next attempt starts empty.
  const [lastError, setLastError] = useState(error);
  if (lastError !== error) {
    setLastError(error);
    if (error !== null) setInput("");
  }

  const valid = legacy ? /^\d{6}$/.test(input) : normalizeRecoveryCode(input) !== null;
  return (
    <EncryptionFrame
      open={open}
      title={t("e2ee.unlock.title")}
      detail={t(legacy ? "e2ee.migrate.unlock" : "e2ee.unlock.detail")}
      onClose={onClose}
      closeDisabled={pending}
      onSubmit={() => {
        if (dead) onForgot();
        else if (valid && !pending) onSubmit(input);
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
            <Button type="submit" variant="primary" disabled={!valid || pending}>
              {pending ? t("e2ee.unlock.working") : t("e2ee.unlock.submit")}
            </Button>
          )}
        </>
      }
    >
      {dead ? null : legacy ? (
        <PinInput
          value={input}
          onChange={setInput}
          label={t("e2ee.pin.label")}
          invalid={error !== null}
          disabled={pending}
          {...(error === null ? {} : { describedBy: errorId })}
        />
      ) : (
        <label className="enc-field">
          <span className="fld-lab">{t("e2ee.recovery.label")}</span>
          <input className="set-input" type="password" autoComplete="off" spellCheck={false}
            value={input} onChange={(e) => setInput(e.target.value)} disabled={pending} />
        </label>
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

export interface LostRecoveryCodeDialogProps {
  open: boolean;
  onContinue(): void;
  onClose(): void;
}

/** What a reset costs, said before it starts. */
export function LostRecoveryCodeDialog({ open, onContinue, onClose }: LostRecoveryCodeDialogProps) {
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
