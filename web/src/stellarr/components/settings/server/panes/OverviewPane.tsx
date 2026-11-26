import { useEffect, useId, useRef, useState } from "react";

import { Button, Dropdown, cx } from "@stellarr/ui";
import { ServerTile } from "@/components/auth/Faces";
import { Avatar } from "@/components/primitives/Avatar";
import { ConfirmError } from "@/components/primitives/ConfirmDialog";
// The transfer is asked on the question card, so it wears the card's parts.
import card from "@/components/primitives/ConfirmDialog.module.css";
import { Modal } from "@/components/primitives/Modal";
import { Switch } from "@/components/primitives/Switch";
import { AvatarUpload } from "@/components/settings/AvatarUpload";
import { countedNoun, formatShortDate } from "@/components/settings/paneFormat";
import { SettingsSaveBar } from "@/components/settings/SettingsSaveBar";
import { SettingsSection } from "@/components/settings/SettingsSection";
import { ServerMark } from "@/components/settings/server/panes/ServerMark";
import { useAsyncAction } from "@/hooks/useAsyncAction";
import type { ServerPatch } from "@/ports";
import { initials } from "@/lib/format/format";
import { useLanguage } from "@/state/hooks";

const SAVED_FLASH_MS = 1500;

/** Someone the server can be handed to. */
export interface TransferCandidate {
  id: string;
  name: string;
  /** Their role, said beside the name in the picker. */
  role?: string;
}

export interface OverviewPaneProps {
  supported: boolean;
  iconSupported: boolean;
  name: string;
  /** Two-letter mark, drawn when the server has no icon. */
  mark: string;
  iconUrl?: string;
  /** ISO 8601. Absent only in legacy development fixtures. */
  createdAt?: string;
  memberCount: number;
  channelCount: number;
  description: string;
  isPublic: boolean;
  onSave: (patch: ServerPatch) => Promise<void>;
  onPickIcon: (file: File) => Promise<void>;
  /** Who owns the server, under Ownership. */
  owner?: { id: string; displayName: string; avatarUrl?: string };
  /** Whether the reader is the owner, which words the row as theirs. */
  viewerIsOwner?: boolean;
  /** Given to the owner only: hands the server to one of the candidates. */
  transfer?: {
    candidates: readonly TransferCandidate[];
    onTransfer: (userId: string) => Promise<void>;
  };
}

interface Profile {
  name: string;
  description: string;
}

const sameProfile = (a: Profile, b: Profile) => a.name === b.name && a.description === b.description;

/**
 * What the server is, laid out as the user's profile is: the card others see
 * at the top, then the fields that make it. Name and description are a draft
 * with the save bar; visibility is a switch and saves on its own, as every
 * switch in settings does.
 */
export function OverviewPane({
  supported,
  iconSupported,
  name,
  mark,
  iconUrl,
  createdAt,
  memberCount,
  channelCount,
  description,
  isPublic,
  onSave,
  onPickIcon,
  owner,
  viewerIsOwner = false,
  transfer,
}: OverviewPaneProps) {
  const { locale, t } = useLanguage();
  const saveAction = useAsyncAction();
  const visibility = useAsyncAction();
  const [saved, setSaved] = useState(false);
  const [transferOpen, setTransferOpen] = useState(false);
  const savedTimer = useRef<number | undefined>(undefined);
  const nameId = useId();
  const descriptionId = useId();

  // Typing needs no round trip, so the fields are a local draft. It is re-seeded
  // when the saved profile changes underneath, which is what a rejected save
  // looks like from here. Comparing during render, rather than in an effect,
  // paints the re-seeded fields in one pass.
  const profile: Profile = { name, description };
  const [draft, setDraft] = useState(profile);
  const [seed, setSeed] = useState(profile);
  if (!sameProfile(seed, profile)) {
    setSeed(profile);
    setDraft(profile);
  }

  useEffect(() => () => window.clearTimeout(savedTimer.current), []);

  const save = async () => {
    const stored = await saveAction.run(() =>
      onSave({ name: draft.name.trim(), description: draft.description.trim() }),
    );
    if (!stored) return;
    setSaved(true);
    window.clearTimeout(savedTimer.current);
    savedTimer.current = window.setTimeout(() => setSaved(false), SAVED_FLASH_MS);
  };

  const dirty = draft.name.trim() !== name.trim() || draft.description.trim() !== description.trim();
  const previewName = draft.name.trim() || name;
  const previewDescription = draft.description.trim();

  return (
    <>
      {supported ? null : <p className="set-note">{t("settings.serverUnsupported")}</p>}

      {/* The server as a person with its link meets it, following the draft
          before it is saved: the invite page's tile, the name, the size and
          the description, and a join that does nothing here. */}
      <section className="ss-meet" aria-label={t("srv.preview")}>
        <div className="ss-meet-card" aria-hidden="true">
          <span className="ss-meet-k">{t("srv.invitePreview.label")}</span>
          <div className="ss-meet-id">
            <ServerTile mark={initials(previewName)} iconUrl={iconUrl ?? null} />
            <span>
              <b>{previewName}</b>
              <span>{countedNoun(t, locale, "settings.member", memberCount)}</span>
            </span>
          </div>
          <p className={previewDescription === "" ? "empty" : undefined}>
            {previewDescription === "" ? t("srv.noDescription") : previewDescription}
          </p>
          <Button type="button" variant="primary" tabIndex={-1}>
            {t("shell.joinServer")}
          </Button>
        </div>
        <div className="ss-meet-txt">
          <b>{t("srv.invitePreview.title")}</b>
          <span>{t("srv.invitePreview.detail")}</span>
          <div className="ss-card-meta">
            <span>{countedNoun(t, locale, "settings.member", memberCount)}</span>
            <span>{countedNoun(t, locale, "settings.channel", channelCount)}</span>
            <span className={isPublic ? "on" : undefined}>{isPublic ? t("srv.public") : t("srv.inviteOnly")}</span>
            {createdAt === undefined ? null : (
              <span>{t("srv.since", { date: formatShortDate(createdAt, locale) })}</span>
            )}
          </div>
        </div>
      </section>
      <SettingsSection title={t("srv.profile")}>
        <div className="frm">
          <div className="frm-row">
            <label className="frm-lab" htmlFor={nameId}>
              {t("srv.name")}
              <span>{t("settings.serverNameDetail")}</span>
            </label>
            <div className="frm-ctl">
              <input
                id={nameId}
                className="set-input"
                type="text"
                value={draft.name}
                maxLength={80}
                disabled={!supported}
                onChange={(event) => setDraft({ ...draft, name: event.target.value })}
              />
            </div>
          </div>
          <div className="frm-row">
            <span className="frm-lab">
              {t("srv.icon")}
              <span>{t("srv.iconDetail")}</span>
            </span>
            <div className="frm-ctl">
              <div className="ss-icon-ctl">
                <AvatarUpload
                  label={t(iconUrl === undefined ? "srv.iconAdd" : "srv.iconChange")}
                  shape="square"
                  disabled={!iconSupported}
                  onSave={onPickIcon}
                >
                  <ServerMark mark={mark} iconUrl={iconUrl} />
                </AvatarUpload>
              </div>
            </div>
          </div>
          <div className="frm-row">
            <label className="frm-lab" htmlFor={descriptionId}>
              {t("settings.description")}
              <span>{t("srv.descriptionDetail")}</span>
            </label>
            <div className="frm-ctl">
              <textarea
                id={descriptionId}
                className="set-area"
                placeholder={t("settings.serverDescriptionPlaceholder")}
                value={draft.description}
                maxLength={280}
                disabled={!supported}
                onChange={(event) => setDraft({ ...draft, description: event.target.value })}
              />
              <span className="ss-count">{`${draft.description.length}/280`}</span>
            </div>
          </div>
        </div>
        {saveAction.failed ? (
          <p className="set-note sound-error" role="alert">
            {t("settings.serverSaveFailed")}
          </p>
        ) : null}
      </SettingsSection>

      <SettingsSection title={t("srv.visibility")}>
        <div className="frm">
          {/* The label is the on state, not a category: "Visibility" would not
              say which way the switch has to be turned. */}
          <div className="frm-row">
            <span className="frm-lab">
              {t("settings.publicServer")}
              <span>{t("srv.publicDetail")}</span>
            </span>
            <div className="frm-ctl frm-switch">
              <Switch
                checked={isPublic}
                onChange={(checked) => void visibility.run(() => onSave({ isPublic: checked }))}
                label={t("settings.publicServer")}
                disabled={!supported || visibility.pending}
              />
            </div>
          </div>
        </div>
        {visibility.failed ? (
          <p className="set-note sound-error" role="alert">
            {t("srv.visibilityFailed")}
          </p>
        ) : null}
      </SettingsSection>

      {owner === undefined ? null : (
        <SettingsSection title={t("srv.ownership")} bodyClassName="sec-list">
          <div className="sec-row">
            <Avatar id={owner.id} displayName={owner.displayName} avatarUrl={owner.avatarUrl} size="sm" />
            <span className="sec-copy">
              <b>{viewerIsOwner ? t("srv.ownerYou") : t("srv.ownerIs", { name: owner.displayName })}</b>
              <small>{t("srv.ownerDetail")}</small>
            </span>
            {transfer === undefined ? null : (
              <Button
                type="button"
                variant="secondary"
                size="sm"
                disabled={transfer.candidates.length === 0}
                onClick={() => setTransferOpen(true)}
              >
                {t("srv.transfer.button")}
              </Button>
            )}
          </div>
        </SettingsSection>
      )}

      {transfer === undefined ? null : (
        <TransferDialog
          open={transferOpen}
          candidates={transfer.candidates}
          onTransfer={transfer.onTransfer}
          onClose={() => setTransferOpen(false)}
        />
      )}

      <SettingsSaveBar
        dirty={dirty}
        saving={saveAction.pending}
        saved={saved}
        saveDisabled={!supported || draft.name.trim() === ""}
        onSave={() => void save()}
        onReset={() => setDraft(profile)}
      />
    </>
  );
}

/** Picking the next owner, in the app's confirm card. */
function TransferDialog({
  open,
  candidates,
  onTransfer,
  onClose,
}: {
  open: boolean;
  candidates: readonly TransferCandidate[];
  onTransfer: (userId: string) => Promise<void>;
  onClose: () => void;
}) {
  const { t } = useLanguage();
  const titleId = useId();
  const labelId = useId();
  const action = useAsyncAction();
  const [picked, setPicked] = useState("");

  // A reopened card starts with nobody picked.
  const [wasOpen, setWasOpen] = useState(open);
  if (wasOpen !== open) {
    setWasOpen(open);
    if (open) setPicked("");
  }

  return (
    <Modal open={open} onClose={onClose} labelledBy={titleId} className={cx(card.wrap)} exitMs={200}>
      <form
        className={card.card}
        noValidate
        onSubmit={(event) => {
          event.preventDefault();
          if (picked === "") return;
          void action.run(() => onTransfer(picked)).then((done) => {
            if (done) onClose();
          });
        }}
      >
        <div className={card.body}>
          <h2 id={titleId}>{t("srv.transfer.title")}</h2>
          <p className={card.sub}>{t("srv.transfer.detail")}</p>
          <div className={card.field}>
            <span className={card.label} id={labelId}>
              {t("srv.transfer.pick")}
            </span>
            <Dropdown
              options={candidates.map((candidate) => ({
                value: candidate.id,
                label: candidate.name,
                ...(candidate.role === undefined ? {} : { detail: candidate.role }),
              }))}
              value={picked}
              onChange={setPicked}
              label={t("srv.transfer.pick")}
              placeholder={t("srv.transfer.placeholder")}
              disabled={action.pending}
            />
          </div>
        </div>
        {action.failed ? <ConfirmError text={t("srv.transfer.failed")} /> : null}
        <div className={card.foot}>
          <Button type="button" variant="secondary" onClick={onClose}>
            {t("common.cancel")}
          </Button>
          <Button type="submit" variant="primary" disabled={picked === "" || action.pending}>
            {t("srv.transfer.confirm")}
          </Button>
        </div>
      </form>
    </Modal>
  );
}
