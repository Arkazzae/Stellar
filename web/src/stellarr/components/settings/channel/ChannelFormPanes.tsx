import { useEffect, useId, useRef, useState } from "react";

import { Icon } from "@stellarr/icons";
import { SettingsSaveBar } from "@/components/settings/SettingsSaveBar";
import { SettingsSection } from "@/components/settings/SettingsSection";
import {
  channelIcon,
  railNeighbours,
  type ChannelSettingsVM,
  type RailGroup,
} from "@/components/settings/channel/channelSettingsModel";
import { useAsyncAction } from "@/hooks/useAsyncAction";
import type { MessageKey, Translator } from "@/i18n";
import type { ChannelKind, ChannelPatch } from "@/ports";
import { useLanguage } from "@/state/hooks";

const SAVED_FLASH_MS = 1500;

/** The slow-mode steps, in the words this app uses. 0 is off. */
const SLOW_MODE: readonly { seconds: number; label: MessageKey }[] = [
  { seconds: 0, label: "channel.slowMode.off" },
  { seconds: 5, label: "channel.slowMode.5" },
  { seconds: 10, label: "channel.slowMode.10" },
  { seconds: 30, label: "channel.slowMode.30" },
  { seconds: 60, label: "channel.slowMode.60" },
  { seconds: 300, label: "channel.slowMode.300" },
  { seconds: 900, label: "channel.slowMode.900" },
  { seconds: 3600, label: "channel.slowMode.3600" },
];

/** How long a quiet channel stays in the rail. 0 means "always". */
const HIDE_AFTER: readonly { hours: number; label: MessageKey }[] = [
  { hours: 0, label: "channel.hideAfter.0" },
  { hours: 1, label: "channel.hideAfter.1" },
  { hours: 24, label: "channel.hideAfter.24" },
  { hours: 72, label: "channel.hideAfter.72" },
  { hours: 168, label: "channel.hideAfter.168" },
];

/** How many people the call takes. 0 means no limit. */
const USER_LIMITS: readonly number[] = [0, 2, 4, 6, 8, 10, 16, 25, 50];

/** How many of a group's channels the preview draws around the one it shows. */
const NEIGHBOURS = 2;

/** A rail caption: a section's name, or the kind of the channels outside sections. */
export function groupCaption(t: Translator, group: Pick<RailGroup, "section" | "kind">): string {
  if (group.section !== undefined) return group.section.name;
  return t(group.kind === "voice" ? "shell.voiceChannels" : "shell.textChannels");
}

/** One channel row of a preview's rail slice. */
export function PreviewRow({
  kind,
  name,
  state,
}: {
  kind: ChannelKind;
  name: string;
  /** "on" is the selected row, "dim" a neighbour drawn only for context. */
  state?: "on" | "dim";
}) {
  return (
    <div className={state === undefined ? "ch-place-row" : `ch-place-row ${state}`}>
      <span className="ch-place-mark">{kind === "voice" ? <Icon name="speaker" size={14} /> : "#"}</span>
      <span className="ch-place-name">{name}</span>
    </div>
  );
}

/**
 * A caption of a preview's rail slice, with the rail's own letters: "on" for
 * the one being edited, "dim" for a neighbour drawn only for context.
 */
export function PreviewCaption({ text, state }: { text: string; state?: "on" | "dim" }) {
  return (
    <div className={state === undefined ? "ch-place-cap" : `ch-place-cap ${state}`}>
      <span className="ch-place-caption group-caption">{text}</span>
    </div>
  );
}

export interface ChannelPreviewProps {
  channel: ChannelSettingsVM;
  /** The name and topic being typed, which the preview shows before they are saved. */
  name: string;
  topic: string;
}

/**
 * The channel where it lives, while it is being edited: its row selected in
 * a slice of the rail among its neighbours, and the room's header with the
 * name and the topic the way the real header separates them. A voice channel
 * has the speaker for a mark and no topic.
 *
 * It is a picture of what the form below says, so it is hidden from
 * assistive tech, which reads the form itself.
 */
export function ChannelPreview({ channel, name, topic }: ChannelPreviewProps) {
  const { t } = useLanguage();
  const voice = channel.kind === "voice";
  const rail = channel.rail ?? [];
  const index = rail.findIndex((group) => group.channels.some((item) => item.id === channel.id));
  const group = rail[index];
  const { after } = railNeighbours(rail, index);

  // The channel and up to two neighbours on either side of it, in rail order.
  const rows = group?.channels ?? [{ id: channel.id, name: channel.name, kind: channel.kind }];
  const at = Math.max(
    rows.findIndex((item) => item.id === channel.id),
    0,
  );
  const shown = rows.slice(Math.max(at - NEIGHBOURS, 0), at + NEIGHBOURS + 1);
  const caption =
    group === undefined
      ? (channel.sectionName ?? t(voice ? "shell.voiceChannels" : "shell.textChannels"))
      : groupCaption(t, group);
  const shownName = name.trim() || channel.name;
  const shownTopic = topic.trim();

  return (
    <div className="pv" aria-hidden="true">
      <div className="pv-rail">
        <PreviewCaption text={caption} />
        {shown.map((item) =>
          item.id === channel.id ? (
            <PreviewRow key={item.id} kind={channel.kind} name={shownName} state="on" />
          ) : (
            <PreviewRow key={item.id} kind={item.kind} name={item.name} />
          ),
        )}
        {after === undefined || after.channels[0] === undefined ? null : (
          <>
            <PreviewCaption text={groupCaption(t, after)} state="dim" />
            <PreviewRow kind={after.channels[0].kind} name={after.channels[0].name} state="dim" />
          </>
        )}
      </div>
      <div className="pv-room">
        <div className="pv-head">
          <span className="pv-glyph">
            <Icon name={channelIcon(channel.kind)} size={18} />
          </span>
          <b>{shownName}</b>
          {voice || shownTopic === "" ? null : <span className="pv-topic">{shownTopic}</span>}
        </div>
        <div className="pv-lines">
          <i style={{ width: "62%" }} />
          <i style={{ width: "84%" }} />
          <i style={{ width: "48%" }} />
        </div>
      </div>
    </div>
  );
}

/** The settings that shape how a channel behaves, beside its name. */
export interface ChannelBehaviour {
  topic: string;
  slowModeSeconds: number;
  hideAfterInactivityHours: number;
  userLimit: number;
}

export interface ChannelBehaviourRowsProps {
  kind: ChannelKind;
  value: ChannelBehaviour;
  onChange: (part: Partial<ChannelBehaviour>) => void;
  disabled: boolean;
}

/**
 * The form rows for a channel's behaviour: topic and slow mode for a text
 * channel, the person limit for a voice one, and when it leaves the rail.
 */
export function ChannelBehaviourRows({ kind, value, onChange, disabled }: ChannelBehaviourRowsProps) {
  const { t } = useLanguage();
  const topicId = useId();
  const slowId = useId();
  const limitId = useId();
  const hideId = useId();

  return (
    <>
      {kind === "voice" ? (
        <div className="frm-row">
          <label className="frm-lab" htmlFor={limitId}>
            {t("channel.personLimit")}
            <span>{t("channel.personLimitDetail")}</span>
          </label>
          <div className="frm-ctl">
            <select
              id={limitId}
              className="set-input"
              value={value.userLimit}
              disabled={disabled}
              onChange={(event) => onChange({ userLimit: Number(event.target.value) })}
            >
              {USER_LIMITS.map((limit) => (
                <option key={limit} value={limit}>
                  {limit === 0
                    ? t("channel.unlimited")
                    : t("channel.personLimitValue", { count: limit })}
                </option>
              ))}
            </select>
          </div>
        </div>
      ) : (
        <>
          <div className="frm-row">
            <label className="frm-lab" htmlFor={topicId}>
              {t("channel.topic")}
              <span>{t("channel.topicDetail")}</span>
            </label>
            <div className="frm-ctl">
              <textarea
                id={topicId}
                className="set-area"
                placeholder={t("channel.topicPlaceholder")}
                value={value.topic}
                maxLength={280}
                disabled={disabled}
                onChange={(event) => onChange({ topic: event.target.value })}
              />
            </div>
          </div>

          <div className="frm-row">
            <label className="frm-lab" htmlFor={slowId}>
              {t("channel.slowMode")}
              <span>{t("channel.slowModeDetail")}</span>
            </label>
            <div className="frm-ctl">
              <select
                id={slowId}
                className="set-input"
                value={value.slowModeSeconds}
                disabled={disabled}
                onChange={(event) => onChange({ slowModeSeconds: Number(event.target.value) })}
              >
                {SLOW_MODE.map((step) => (
                  <option key={step.seconds} value={step.seconds}>
                    {t(step.label)}
                  </option>
                ))}
              </select>
            </div>
          </div>
        </>
      )}

      <div className="frm-row">
        <label className="frm-lab" htmlFor={hideId}>
          {t("channel.hideAfterInactivity")}
          <span>{t("channel.hideAfterInactivityDetail")}</span>
        </label>
        <div className="frm-ctl">
          <select
            id={hideId}
            className="set-input"
            value={value.hideAfterInactivityHours}
            disabled={disabled}
            onChange={(event) =>
              onChange({ hideAfterInactivityHours: Number(event.target.value) })
            }
          >
            {HIDE_AFTER.map((step) => (
              <option key={step.hours} value={step.hours}>
                {t(step.label)}
              </option>
            ))}
          </select>
        </div>
      </div>
    </>
  );
}

interface Draft extends ChannelBehaviour {
  name: string;
}

const draftOf = (channel: ChannelSettingsVM): Draft => ({
  name: channel.name,
  topic: channel.topic,
  slowModeSeconds: channel.slowModeSeconds,
  hideAfterInactivityHours: channel.hideAfterInactivityHours,
  userLimit: channel.userLimit,
});

const sameDraft = (a: Draft, b: Draft) =>
  a.name === b.name &&
  a.topic === b.topic &&
  a.slowModeSeconds === b.slowModeSeconds &&
  a.hideAfterInactivityHours === b.hideAfterInactivityHours &&
  a.userLimit === b.userLimit;

export interface ChannelOverviewPaneProps {
  channel: ChannelSettingsVM;
  supported: boolean;
  onSave: (patch: ChannelPatch) => Promise<void>;
}

/**
 * Everything a channel is called and how it behaves, in one card with its own
 * save, the way the server overview keeps the server's profile.
 */
export function ChannelOverviewPane({ channel, supported, onSave }: ChannelOverviewPaneProps) {
  const { t } = useLanguage();
  const saveAction = useAsyncAction();
  const [saved, setSaved] = useState(false);
  const savedTimer = useRef<number | undefined>(undefined);
  const nameId = useId();
  const voice = channel.kind === "voice";

  // Typing needs no round trip, so the fields are a local draft. It is re-seeded
  // when the saved channel changes underneath: another channel, or this one
  // after a save. Comparing during render paints the new fields in one pass.
  const current = draftOf(channel);
  const [draft, setDraft] = useState(current);
  const [seed, setSeed] = useState(current);
  if (!sameDraft(seed, current)) {
    setSeed(current);
    setDraft(current);
  }

  useEffect(() => () => window.clearTimeout(savedTimer.current), []);

  const patch = (part: Partial<Draft>) => setDraft((value) => ({ ...value, ...part }));

  // Disabled until something changed: a button that is always live cannot tell
  // the reader whether they have unsaved work.
  const dirty =
    draft.name.trim() !== channel.name ||
    draft.hideAfterInactivityHours !== channel.hideAfterInactivityHours ||
    (voice
      ? draft.userLimit !== channel.userLimit
      : draft.topic.trim() !== channel.topic || draft.slowModeSeconds !== channel.slowModeSeconds);

  const save = async () => {
    const stored = await saveAction.run(() =>
      onSave({
        name: draft.name.trim(),
        hideAfterInactivityHours: draft.hideAfterInactivityHours,
        ...(voice
          ? { userLimit: draft.userLimit }
          : { topic: draft.topic.trim(), slowModeSeconds: draft.slowModeSeconds }),
      }),
    );
    if (!stored) return;
    setSaved(true);
    window.clearTimeout(savedTimer.current);
    savedTimer.current = window.setTimeout(() => setSaved(false), SAVED_FLASH_MS);
  };

  return (
    <>
    <ChannelPreview channel={channel} name={draft.name} topic={draft.topic} />
    <SettingsSection title={t("channel.channel")} className="overview-section">
      <div className="frm">
        <div className="frm-row">
          <label className="frm-lab" htmlFor={nameId}>
            {t("channel.name")}
            <span>{t("channel.nameDetail")}</span>
          </label>
          <div className="frm-ctl">
            <input
              id={nameId}
              className="set-input"
              type="text"
              value={draft.name}
              maxLength={60}
              disabled={!supported}
              onChange={(event) => patch({ name: event.target.value })}
            />
          </div>
        </div>

        <ChannelBehaviourRows
          kind={channel.kind}
          value={draft}
          onChange={patch}
          disabled={!supported}
        />
      </div>

      {saveAction.failed ? (
        <p className="set-note sound-error ov-save-error" role="alert">
          {t("channel.settingsSaveFailed")}
        </p>
      ) : null}

      <SettingsSaveBar
        dirty={dirty}
        saving={saveAction.pending}
        saved={saved}
        saveDisabled={!supported}
        onSave={() => void save()}
        onReset={() => setDraft(current)}
      />
    </SettingsSection>
    </>
  );
}
