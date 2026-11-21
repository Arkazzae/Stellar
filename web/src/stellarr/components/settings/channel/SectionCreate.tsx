import { useEffect, useId, useRef, useState } from "react";

import { Icon } from "@stellarr/icons";
import { CreateDialog } from "@/components/settings/CreateDialog";
import {
  sectionChannels,
  type RailChannel,
} from "@/components/settings/channel/channelSettingsModel";
import { useLanguage } from "@/state/hooks";

export interface SectionCreateProps {
  open: boolean;
  onClose: () => void;
  /** The server the section is made in, at the top of the rail slice. */
  serverName: string;
  /** The server's sections in rail order; the new one joins the end. */
  sections: readonly { id: string; name: string }[];
  /** The server's channels in rail order, one of each group drawn for context. */
  channels: readonly RailChannel[];
  onCreate: (input: { name: string }) => Promise<void>;
}

/** One caption of the slice, with the first channel under it when it has one. */
interface Group {
  key: string;
  caption: string;
  channel?: RailChannel;
}

/**
 * "Add section" in the rail, as a slice of the rail like the new channel: the
 * server's groups in their order, and the new section typed as its caption
 * where it will stand, at the end. Enter adds it.
 */
export function SectionCreate({
  open,
  onClose,
  serverName,
  sections,
  channels,
  onCreate,
}: SectionCreateProps) {
  const { t } = useLanguage();
  const errorId = useId();
  const noteId = useId();
  const inputRef = useRef<HTMLInputElement>(null);
  const [name, setName] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Reset while rendering, not in an effect, so a reopened dialog starts blank.
  const [wasOpen, setWasOpen] = useState(open);
  if (wasOpen !== open) {
    setWasOpen(open);
    setError(null);
    if (!open) {
      setName("");
      setSubmitting(false);
    }
  }

  // The name is the only answer, so it takes the focus once the dialog opens;
  // a child's effects run first, which puts this after the modal's own focus.
  useEffect(() => {
    if (open) inputRef.current?.focus();
  }, [open]);

  const trimmed = name.trim();

  // The sections come before the new one; a server with none yet shows its
  // two groups of unsectioned channels instead, so the slice is never bare.
  const groups: Group[] =
    sections.length > 0
      ? sections.map((section) => ({
          key: section.id,
          caption: section.name,
          channel: sectionChannels(channels, section.id)[0],
        }))
      : (["text", "voice"] as const).map((kind) => ({
          key: kind,
          caption: t(kind === "text" ? "shell.textChannels" : "shell.voiceChannels"),
          channel: channels.find((item) => item.kind === kind && item.sectionId === undefined),
        }));

  const submit = async () => {
    if (submitting || trimmed.length === 0) return;
    setSubmitting(true);
    setError(null);
    try {
      await onCreate({ name: trimmed });
      onClose();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : t("section.createFailed"));
      inputRef.current?.focus();
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <CreateDialog
      open={open}
      onClose={onClose}
      title={t("section.newTitle")}
      detail={t("section.newDetail")}
      submitLabel={t("section.new")}
      busy={submitting}
      busyLabel={t("section.creating")}
      canSubmit={trimmed.length > 0}
      onSubmit={() => void submit()}
      className="ch-place-modal"
      notes={
        <>
          <p id={noteId} className="ch-place-hint">
            {t("section.moveHint")}
          </p>
          {error === null ? null : (
            <p id={errorId} className="ch-place-error" role="alert">
              {error}
            </p>
          )}
        </>
      }
    >
      <div className="ch-place" role="group" aria-label={t("section.placement")}>
        <div className="ch-place-server">{serverName}</div>
        {groups.map((group) => (
          <div key={group.key} className="ch-place-group">
            <div className="ch-place-cap">
              <span className="ch-place-caption group-caption">{group.caption}</span>
            </div>
            {group.channel === undefined ? null : (
              <div className="ch-place-row dim">
                <span className="ch-place-mark" aria-hidden="true">
                  {group.channel.kind === "voice" ? <Icon name="speaker" size={14} /> : "#"}
                </span>
                <span className="ch-place-name">{group.channel.name}</span>
              </div>
            )}
          </div>
        ))}
        <div className="ch-place-group">
          {/* The name is typed as the caption it becomes: the rail's letters. */}
          <div className="ch-place-cap new" data-locked={submitting || undefined}>
            <input
              ref={inputRef}
              className="ch-place-input group-caption"
              type="text"
              value={name}
              maxLength={60}
              placeholder={t("section.name")}
              aria-label={t("section.name")}
              aria-invalid={error !== null || undefined}
              aria-describedby={error === null ? noteId : `${noteId} ${errorId}`}
              disabled={submitting}
              required
              autoComplete="off"
              spellCheck={false}
              onChange={(event) => {
                setName(event.target.value);
                setError(null);
              }}
            />
          </div>
          <div className="ch-place-row empty">{t("section.emptyYet")}</div>
        </div>
      </div>
    </CreateDialog>
  );
}
