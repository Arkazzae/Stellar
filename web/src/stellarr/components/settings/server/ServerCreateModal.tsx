import { useEffect, useId, useRef, useState, type FormEvent } from "react";

import { Button } from "@stellarr/ui";
import { Icon } from "@stellarr/icons";
import { ServerTile } from "@/components/auth/Faces";
import { DialogClose } from "@/components/primitives/DialogClose";
import { Sticker } from "@/components/primitives/MascotArt";
import { Modal } from "@/components/primitives/Modal";
import { Segmented } from "@/components/primitives/Segmented";
import { StepIndicator } from "@/components/primitives/StepIndicator";
import { Switch } from "@/components/primitives/Switch";
import { useRovingRadio } from "@/components/primitives/useRovingRadio";
import { countedNoun } from "@/components/settings/paneFormat";
import type { InterfaceLocale, Translator } from "@/i18n";
import { initials, toChannelSlug } from "@/lib/format/format";
import { useLanguage } from "@/state/hooks";

import styles from "./ServerCreateModal.module.css";

export interface ServerCreateInput {
  name: string;
  description: string;
  isPublic: boolean;
  /** The first text channel, already slugged. */
  textChannelName: string;
  /** The first voice channel, already slugged; empty creates none. */
  voiceChannelName: string;
  mentionsOnly: boolean;
  voiceJoinSound: boolean;
}

/** A server already on the rail, as the preview draws its tile. */
export interface RailServer {
  id: string;
  name: string;
  iconUrl?: string;
}

/** The invite link made for the new server, as it comes. */
export type CreatedServerInvite =
  | { state: "making" }
  | { state: "ready"; url: string }
  | { state: "failed" };

export interface ServerCreateModalProps {
  open: boolean;
  onClose: () => void;
  /**
   * Creates the server. Once it resolves the app switches to the new server,
   * which mounts the app again, so the dialog stays busy until then.
   */
  onCreate: (input: ServerCreateInput) => Promise<void>;
  /** The servers already on the rail, which the preview draws before the new one. */
  servers?: readonly RailServer[];
  /**
   * The server this dialog created, once the app has switched to it. The
   * dialog then shows the invite screen for it, with the link in `invite`.
   */
  created?: { name: string; description?: string } | null;
  invite?: CreatedServerInvite;
  onCopyInvite?: (url: string) => Promise<void>;
}

type Step = 1 | 2 | 3;

interface Draft {
  name: string;
  description: string;
  isPublic: boolean;
  textChannel: string;
  voiceChannel: string;
  withVoice: boolean;
  mentionsOnly: boolean;
  voiceJoinSound: boolean;
}

/** The channel names are suggestions in the reader's language, kept if left alone. */
function blankDraft(t: Translator): Draft {
  return {
    name: "",
    description: "",
    isPublic: false,
    textChannel: t("shell.starterTextChannelName"),
    voiceChannel: t("shell.starterVoiceChannelName"),
    withVoice: true,
    mentionsOnly: true,
    voiceJoinSound: true,
  };
}

/** The module's classes for the names given, skipping the ones switched off. */
function css(...names: (string | false)[]): string {
  return names
    .map((name) => (name === false ? undefined : styles[name]))
    .filter(Boolean)
    .join(" ");
}

/** The rail's own tiles that the preview shows above the new one. */
const PREVIEW_TILES = 4;

/**
 * Creating a server in three short steps: its name, its first channels, and
 * who may join. Each step can create the server at once with what is filled
 * in so far; what was never reached keeps its default. A preview beside the
 * form shows the server where it will live: the rail while it is named and
 * given channels, and the invite card once it is about who can join.
 *
 * After the server is made the app switches to it, and the same dialog comes
 * back over the new server with a link to invite people.
 */
export function ServerCreateModal({
  open,
  onClose,
  onCreate,
  servers = [],
  created = null,
  invite = { state: "making" },
  onCopyInvite,
}: ServerCreateModalProps) {
  const { t, locale } = useLanguage();
  const titleId = useId();
  const nameId = useId();
  const nameNoteId = useId();
  const descriptionId = useId();
  const textChannelId = useId();
  const textNoteId = useId();
  const voiceNoteId = useId();
  const accessIds = useId();
  const nameInput = useRef<HTMLInputElement>(null);
  const textInput = useRef<HTMLInputElement>(null);
  const accessGroup = useRef<HTMLDivElement>(null);
  const [draft, setDraft] = useState(() => blankDraft(t));
  const [step, setStep] = useState<Step>(1);
  // Which button started the creation, so that one says it is working.
  const [submitting, setSubmitting] = useState<"now" | "create" | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);

  // Reset while rendering, not in an effect, so a reopened dialog is never
  // painted once with the previous draft. The draft is dropped on closing and
  // renewed on opening, which keeps the suggested names in the current language.
  const [wasOpen, setWasOpen] = useState(open);
  if (wasOpen !== open) {
    setWasOpen(open);
    setStep(1);
    setError(null);
    setCopied(false);
    setDraft(blankDraft(t));
    if (!open) setSubmitting(null);
  }

  const done = created !== null;
  const busy = submitting !== null && !done;

  const change = <K extends keyof Draft>(key: K, value: Draft[K]) =>
    setDraft((current) => ({ ...current, [key]: value }));

  const name = draft.name.trim();
  const textSlug = toChannelSlug(draft.textChannel);
  const voiceSlug = toChannelSlug(draft.voiceChannel);
  // The server rejects two starter channels sharing a name, so the dialog says
  // so beside the field rather than letting the submit come back with it.
  const channelsClash = draft.withVoice && textSlug.length > 0 && textSlug === voiceSlug;
  const channelsReady =
    textSlug.length > 0 && (!draft.withVoice || voiceSlug.length > 0) && !channelsClash;
  const canMoveOn = name.length > 0 && (step === 1 || channelsReady);

  const access = useRovingRadio(2, draft.isPublic ? 1 : 0, (index) => change("isPublic", index === 1));

  // Each step opens on its first control, since the button that led there may
  // be gone, and a refusal puts the name field under the cursor again.
  const refused = error !== null;
  useEffect(() => {
    if (!open || done) return;
    if (step === 1) nameInput.current?.focus();
    else if (step === 2) textInput.current?.focus();
    else accessGroup.current?.querySelector<HTMLElement>('[aria-checked="true"]')?.focus();
  }, [open, done, step, refused]);

  const close = () => {
    if (!busy) onClose();
  };

  const create = async (by: "now" | "create") => {
    if (busy || name.length === 0) return;
    // "Create now" from the first step after the channels were left clashing:
    // the channels step says what is wrong instead of a refusal from the server.
    if (!channelsReady) {
      setStep(2);
      return;
    }
    setSubmitting(by);
    setError(null);
    try {
      await onCreate({
        name,
        description: draft.description.trim(),
        isPublic: draft.isPublic,
        textChannelName: textSlug,
        voiceChannelName: draft.withVoice ? voiceSlug : "",
        mentionsOnly: draft.mentionsOnly,
        voiceJoinSound: draft.voiceJoinSound,
      });
    } catch (cause) {
      // Whatever step the creation started from, the refusal is about the
      // server, so it is said under its name.
      setStep(1);
      setError(cause instanceof Error ? cause.message : t("shell.serverCreateFailed"));
      setSubmitting(null);
    }
  };

  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (done) onClose();
    else if (step === 3) void create("create");
    else if (canMoveOn) setStep(step === 1 ? 2 : 3);
  };

  const copy = async (url: string) => {
    if (onCopyInvite === undefined) return;
    try {
      await onCopyInvite(url);
      setCopied(true);
    } catch {
      // The link stays on screen to select by hand.
    }
  };

  const stepNames = [
    t("shell.createServerStepServer"),
    t("shell.createServerStepChannels"),
    t("shell.createServerStepAccess"),
  ];

  const renderStep = () => {
    if (created !== null) {
      return (
        <div className={css("done")}>
          <Sticker name="05-invite-friends" size="main" align="center" />
          <h2 id={titleId} className={css("title")}>
            {t("auth.setup.inviteTitle")}
          </h2>
          <p className={css("lead")}>{t("auth.setup.inviteDescription", { server: created.name })}</p>
          {invite.state === "failed" ? (
            <p className={css("hint", "bad", "doneNote")} role="alert">
              {t("auth.setup.inviteFailed")}
            </p>
          ) : (
            <>
              <div className={css("link")}>
                <output className={css("input", "url")} aria-label={t("auth.invite.linkLabel")}>
                  {invite.state === "ready" ? invite.url : t("auth.setup.inviteMaking")}
                </output>
                <Button
                  type="button"
                  variant="secondary"
                  disabled={invite.state !== "ready"}
                  onClick={() => invite.state === "ready" && void copy(invite.url)}
                >
                  <Icon name={copied ? "check" : "copy"} size={16} />
                  {copied ? t("auth.setup.copied") : t("auth.setup.copy")}
                </Button>
              </div>
              <p className={css("hint", "doneNote")}>{t("auth.setup.inviteHint")}</p>
            </>
          )}
        </div>
      );
    }

    switch (step) {
      case 1:
        return (
          <>
            <h2 id={titleId} className={css("title")}>
              {t("shell.createServerNameTitle")}
            </h2>
            <p className={css("lead")}>{t("auth.setup.serverDescription")}</p>
            <div className={css("fields")}>
              <div className={css("field")}>
                <label className={css("label")} htmlFor={nameId}>
                  {t("shell.serverName")}
                </label>
                <input
                  ref={nameInput}
                  id={nameId}
                  className={css("input")}
                  type="text"
                  value={draft.name}
                  maxLength={80}
                  placeholder={t("shell.serverNamePlaceholder")}
                  disabled={busy}
                  required
                  autoComplete="off"
                  aria-invalid={refused || undefined}
                  aria-describedby={nameNoteId}
                  onChange={(event) => {
                    change("name", event.target.value);
                    setError(null);
                  }}
                />
                {error === null ? (
                  <p id={nameNoteId} className={css("hint")}>
                    {t("shell.serverNameDetail")}
                  </p>
                ) : (
                  <p id={nameNoteId} className={css("hint", "bad")} role="alert">
                    {error}
                  </p>
                )}
              </div>
              <div className={css("field")}>
                <label className={css("label")} htmlFor={descriptionId}>
                  {t("shell.serverDescription")}
                  <span className={css("optional")}>{t("shell.createServerOptional")}</span>
                </label>
                <textarea
                  id={descriptionId}
                  className={css("input")}
                  value={draft.description}
                  maxLength={280}
                  placeholder={t("shell.serverDescriptionPlaceholder")}
                  disabled={busy}
                  onChange={(event) => change("description", event.target.value)}
                />
              </div>
            </div>
          </>
        );
      case 2:
        return (
          <>
            <h2 id={titleId} className={css("title")}>
              {t("shell.createServerChannelsTitle")}
            </h2>
            <p className={css("lead")}>{t("shell.starterChannelsNote")}</p>
            <div className={css("fields")}>
              <div className={css("field")}>
                <label className={css("label")} htmlFor={textChannelId}>
                  {t("shell.starterTextChannel")}
                </label>
                <div className={css("channelRow")}>
                  <span className={css("glyph")} aria-hidden="true">
                    #
                  </span>
                  <input
                    ref={textInput}
                    id={textChannelId}
                    className={css("input")}
                    type="text"
                    value={draft.textChannel}
                    maxLength={60}
                    placeholder={t("shell.starterTextChannelName")}
                    disabled={busy}
                    required
                    autoComplete="off"
                    aria-describedby={textNoteId}
                    onChange={(event) => change("textChannel", event.target.value)}
                  />
                </div>
                <p id={textNoteId} className={css("hint")}>
                  {textSlug.length === 0
                    ? t("channel.lowercaseAndHyphens")
                    : t("channel.willCreate", { name: `#${textSlug}` })}
                </p>
              </div>
              <div className={css("switchRow")}>
                <span className={css("switchText")} aria-hidden="true">
                  <b>{t("shell.starterVoiceChannel")}</b>
                  <span>{t("shell.starterVoiceChannelDetail")}</span>
                </span>
                <Switch
                  checked={draft.withVoice}
                  onChange={(checked) => change("withVoice", checked)}
                  label={t("shell.starterVoiceChannel")}
                  disabled={busy}
                />
              </div>
              {draft.withVoice ? (
                <div className={css("field")}>
                  <div className={css("channelRow")}>
                    <span className={css("glyph")} aria-hidden="true">
                      <Icon name="speaker" size={18} />
                    </span>
                    <input
                      className={css("input")}
                      type="text"
                      value={draft.voiceChannel}
                      maxLength={60}
                      placeholder={t("shell.starterVoiceChannelName")}
                      disabled={busy}
                      required
                      autoComplete="off"
                      aria-label={t("shell.createServerVoiceChannelName")}
                      aria-invalid={channelsClash || undefined}
                      aria-describedby={voiceNoteId}
                      onChange={(event) => change("voiceChannel", event.target.value)}
                    />
                  </div>
                  {channelsClash ? (
                    <p id={voiceNoteId} className={css("hint", "bad")} role="alert">
                      {t("shell.starterChannelsClash")}
                    </p>
                  ) : (
                    <p id={voiceNoteId} className={css("hint")}>
                      {voiceSlug.length === 0
                        ? t("channel.lowercaseAndHyphens")
                        : t("channel.willCreate", { name: voiceSlug })}
                    </p>
                  )}
                </div>
              ) : null}
            </div>
          </>
        );
      case 3: {
        const options = [
          {
            isPublic: false,
            label: t("shell.createServerInviteOnly"),
            detail: t("shell.createServerInviteOnlyDetail"),
          },
          {
            isPublic: true,
            label: t("shell.createServerPublic"),
            detail: t("shell.createServerPublicDetail"),
          },
        ];
        return (
          <>
            <h2 id={titleId} className={css("title")}>
              {t("shell.createServerAccessTitle")}
            </h2>
            <p className={css("lead")}>{t("shell.createServerAccessLead")}</p>
            <div
              ref={accessGroup}
              className={css("pick")}
              role="radiogroup"
              aria-label={t("shell.createServerStepAccess")}
            >
              {options.map((option, index) => {
                const on = option.isPublic === draft.isPublic;
                return (
                  <button
                    key={option.label}
                    ref={access.ref(index)}
                    type="button"
                    role="radio"
                    className={css("option", on && "on")}
                    aria-checked={on}
                    aria-labelledby={`${accessIds}-${index}`}
                    aria-describedby={`${accessIds}-${index}-detail`}
                    tabIndex={access.tabIndex(index)}
                    disabled={busy}
                    onKeyDown={(event) => access.onKeyDown(event, index)}
                    onClick={() => change("isPublic", option.isPublic)}
                  >
                    <span className={css("dot")} aria-hidden="true" />
                    <span className={css("optionText")}>
                      <b id={`${accessIds}-${index}`}>{option.label}</b>
                      <span id={`${accessIds}-${index}-detail`}>{option.detail}</span>
                    </span>
                  </button>
                );
              })}
            </div>
            <div className={css("field", "notify")}>
              <span className={css("label")} aria-hidden="true">
                {t("server.notifications")}
              </span>
              <Segmented
                variant="pill"
                className={css("segmented")}
                label={t("server.notifications")}
                value={draft.mentionsOnly ? "mentions" : "all"}
                disabled={busy}
                onChange={(value) => change("mentionsOnly", value === "mentions")}
                options={[
                  { value: "mentions", label: t("shell.notifyMentions") },
                  { value: "all", label: t("shell.notifyAll") },
                ]}
              />
              <p className={css("hint")}>{t("shell.defaultNotificationsDetail")}</p>
            </div>
            <div className={css("switchRow")}>
              <span className={css("switchText")} aria-hidden="true">
                <b>{t("shell.voiceJoinSound")}</b>
                <span>{t("shell.voiceJoinSoundDetail")}</span>
              </span>
              <Switch
                checked={draft.voiceJoinSound}
                onChange={(checked) => change("voiceJoinSound", checked)}
                label={t("shell.voiceJoinSound")}
                disabled={busy}
              />
            </div>
          </>
        );
      }
    }
  };

  const renderFooter = () => {
    if (done) {
      return (
        <>
          <span className={css("gap")} />
          <Button type="submit" variant="primary">
            {t("common.done")}
          </Button>
        </>
      );
    }
    return (
      <>
        {step === 1 ? (
          <Button type="button" variant="quiet" disabled={busy} onClick={close}>
            {t("common.cancel")}
          </Button>
        ) : (
          <Button
            type="button"
            variant="quiet"
            disabled={busy}
            onClick={() => setStep(step === 3 ? 2 : 1)}
          >
            <Icon name="arrowLeft" size={16} />
            {t("shell.createServerBack")}
          </Button>
        )}
        <span className={css("gap")} />
        {step === 3 ? (
          <Button
            type="submit"
            variant="primary"
            disabled={busy || !canMoveOn}
            aria-busy={submitting === "create" || undefined}
          >
            {submitting === "create" ? t("shell.creatingServer") : t("shell.createServer")}
          </Button>
        ) : (
          <>
            <Button
              type="button"
              variant="secondary"
              disabled={busy || !canMoveOn}
              aria-busy={submitting === "now" || undefined}
              onClick={() => void create("now")}
            >
              {submitting === "now" ? t("shell.creatingServer") : t("shell.createServerNow")}
            </Button>
            <Button type="submit" variant="primary" disabled={busy || !canMoveOn}>
              {t("shell.createServerNext")}
            </Button>
          </>
        )}
      </>
    );
  };

  return (
    <Modal open={open} onClose={close} labelledBy={titleId} className={css("wrap")}>
      <form className={css("sheet")} onSubmit={submit} noValidate>
        <div className={css("form")}>
          <div className={css("top")}>
            {done ? (
              <StepIndicator lit={3} label={t("common.done")} name={t("auth.setup.stepInvite")} />
            ) : (
              <StepIndicator
                lit={step}
                label={t("auth.setup.step", { step, steps: 3 })}
                name={stepNames[step - 1] ?? ""}
              />
            )}
            <DialogClose label={t("common.close")} onClose={close} disabled={busy} />
          </div>
          <div className={css("body")} key={done ? "done" : step}>
            {renderStep()}
          </div>
          <footer className={css("foot")}>{renderFooter()}</footer>
        </div>
        <Preview
          t={t}
          locale={locale}
          step={done ? "done" : step}
          servers={servers}
          name={created?.name ?? name}
          description={(created === null ? draft.description : (created.description ?? "")).trim()}
          isPublic={draft.isPublic}
          textChannel={textSlug}
          voiceChannel={draft.withVoice ? voiceSlug : null}
        />
      </form>
    </Modal>
  );
}

interface PreviewProps {
  t: Translator;
  locale: InterfaceLocale;
  step: Step | "done";
  servers: readonly RailServer[];
  name: string;
  description: string;
  isPublic: boolean;
  textChannel: string;
  /** Null when the server gets no voice channel. */
  voiceChannel: string | null;
}

/**
 * The new server where it will be seen: at the end of the server rail with
 * its channels while it is named and given channels, then as the card a link
 * to it opens (or Discover lists, for a public one). The pictures repeat what
 * the form says, so only the caption and the sentence under them are read out.
 */
function Preview({
  t,
  locale,
  step,
  servers,
  name,
  description,
  isPublic,
  textChannel,
  voiceChannel,
}: PreviewProps) {
  // The rail's mark for a named server (`serverMark`); a server with no name
  // yet has an empty tile, as on the first run.
  const mark = name === "" ? null : initials(name);

  if (step === 1 || step === 2) {
    const row = css("miniRow", step === 2 && "on");
    return (
      <aside className={css("side")} aria-label={t("shell.createServerPreview")}>
        <p className={css("sideCaption")} aria-hidden="true">
          {t("shell.createServerPreview")}
        </p>
        <div className={css("mini")} aria-hidden="true">
          <div className={css("miniRail")}>
            {servers.slice(-PREVIEW_TILES).map((server) => (
              <span key={server.id} className={css("miniTile")}>
                {server.iconUrl === undefined ? initials(server.name) : <img src={server.iconUrl} alt="" />}
              </span>
            ))}
            <span className={css("miniTile", "newTile")}>{mark ?? <Icon name="server" size={18} />}</span>
          </div>
          <div className={css("miniChannels")}>
            <div className={css("miniName", name === "" && "empty")}>
              {name === "" ? t("auth.setup.emptyName") : name}
            </div>
            <div className={css("miniCaption")}>{t("shell.textChannels")}</div>
            <div className={row}>
              <span className={css("miniGlyph")}>#</span>
              <span className={css("miniLabel")}>{textChannel || t("auth.setup.channelPlaceholder")}</span>
            </div>
            {voiceChannel === null ? null : (
              <>
                <div className={css("miniCaption")}>{t("shell.voiceChannels")}</div>
                <div className={row}>
                  <span className={css("miniGlyph")}>
                    <Icon name="speaker" size={14} />
                  </span>
                  <span className={css("miniLabel")}>{voiceChannel || t("auth.setup.channelPlaceholder")}</span>
                </div>
              </>
            )}
          </div>
        </div>
        <p className={css("hint")}>
          {step === 1 ? t("shell.createServerPreviewTile") : t("shell.createServerPreviewChannels")}
        </p>
      </aside>
    );
  }

  // Once made, the server is shown as the link being handed out leads to it.
  const listed = isPublic && step === 3;
  return (
    <aside className={css("side")} aria-label={t("shell.createServerPreview")}>
      <p className={css("sideCaption")} aria-hidden="true">
        {t("shell.createServerPreview")}
      </p>
      <div className={css("card")} aria-hidden="true">
        <span className={css("cardKicker")}>
          {listed ? t("shell.createServerPreviewDiscover") : t("shell.createServerPreviewInvite")}
        </span>
        <div className={css("cardId")}>
          <ServerTile mark={mark} />
          <span className={css("cardName")}>
            <b>{name === "" ? t("auth.setup.emptyName") : name}</b>
            <span>{countedNoun(t, locale, "settings.member", 1)}</span>
          </span>
        </div>
        <p className={css("cardAbout", description === "" && "empty")}>
          {description === "" ? t("shell.publicServerNoDescription") : description}
        </p>
        <span className={css("cardJoin")}>{t("shell.joinServer")}</span>
      </div>
      <p className={css("hint")}>
        {listed ? t("shell.createServerPreviewDiscoverNote") : t("shell.createServerPreviewInviteNote")}
      </p>
    </aside>
  );
}
