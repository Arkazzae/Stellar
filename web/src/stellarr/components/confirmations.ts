import type { ConfirmRequest, ConfirmSubject, PromptRequest } from "@/components/primitives/ConfirmDialog";
import { countedNoun } from "@/components/settings/paneFormat";
import type { InterfaceLocale, Translator } from "@/i18n";

/**
 * Every irreversible question the app asks, in one place.
 *
 * The same deed is often reachable from more than one door (a message from its
 * hover toolbar or its context menu, a friendship from the roster or the
 * friends list), and two wordings for one consequence teach readers to stop
 * reading them. Each request is a question followed by its consequence, and
 * the confirming button names the deed rather than agreeing with the dialog.
 *
 * No title names what it is about: Polish inflects a name to fit the question,
 * and a name dropped into it unchanged reads wrong. The thing itself is the
 * subject row under the sentence, drawn the way the app draws it elsewhere.
 *
 * A caller that has the deed at hand passes it as `run`, so the card stays
 * open while it runs and shows the failure itself.
 */

type ChannelKind = "text" | "voice";

/** One kind of subject, so whoever built a channel can read its detail back. */
type SubjectOf<K extends ConfirmSubject["kind"]> = Extract<ConfirmSubject, { kind: K }>;

/* ------------------------------------------------------------- subjects -- */

export { personSubject } from "./personSubject";

/** A channel, with what it is and where it sits: "Text channel in Information". */
export function channelSubject(
  t: Translator,
  channel: { kind: ChannelKind; name: string; sectionName?: string },
): SubjectOf<"channel"> {
  const voice = channel.kind === "voice";
  const detail =
    channel.sectionName === undefined
      ? t(voice ? "channel.voice" : "channel.text")
      : t(voice ? "confirm.voiceChannelIn" : "confirm.textChannelIn", { section: channel.sectionName });
  return { kind: "channel", channelKind: channel.kind, name: channel.name, detail };
}

/** A server by its mark and name, and its size when the counts are at hand. */
export function serverSubject(
  t: Translator,
  locale: InterfaceLocale,
  server: { name: string; mark: string; iconUrl?: string; memberCount?: number; channelCount?: number },
): SubjectOf<"server"> {
  const counts = [
    server.memberCount === undefined ? null : countedNoun(t, locale, "settings.member", server.memberCount),
    server.channelCount === undefined ? null : countedNoun(t, locale, "settings.channel", server.channelCount),
  ].filter((part): part is string => part !== null);
  return {
    kind: "server",
    name: server.name,
    mark: server.mark,
    ...(server.iconUrl === undefined ? {} : { iconUrl: server.iconUrl }),
    ...(counts.length === 0 ? {} : { detail: counts.join(" · ") }),
  };
}

/** A message by its author and its words; one with no words says so. */
export function messageSubject(
  t: Translator,
  message: { authorId: string; authorName: string; authorAvatarUrl?: string; text: string },
): SubjectOf<"message"> {
  const text = message.text.trim();
  return {
    kind: "message",
    authorId: message.authorId,
    authorName: message.authorName,
    ...(message.authorAvatarUrl === undefined ? {} : { authorAvatarUrl: message.authorAvatarUrl }),
    text: text === "" ? t("confirm.messageNoText") : text,
  };
}

/**
 * A section as the rail captions it, with the channels under it. Null
 * channels are ones the client has not heard of, so none are listed and the
 * section is not called empty either.
 */
export function sectionSubject(
  t: Translator,
  section: { name: string; channels: readonly { id: string; name: string; kind: ChannelKind }[] | null },
): SubjectOf<"section"> {
  return {
    kind: "section",
    name: section.name,
    channels: (section.channels ?? []).map((channel) => ({
      id: channel.id,
      name: channel.name,
      channelKind: channel.kind,
    })),
    ...(section.channels === null ? {} : { emptyDetail: t("confirm.sectionEmpty") }),
  };
}

/* ------------------------------------------------------------- requests -- */

export function deleteMessageConfirmation(t: Translator, subject?: ConfirmSubject): ConfirmRequest {
  return {
    title: t("message.delete.title"),
    description: t("message.delete.description"),
    ...(subject === undefined ? {} : { subject }),
    confirmLabel: t("message.action.delete"),
    busyLabel: t("confirm.deleting"),
    failure: t("confirm.messageDeleteFailed"),
    danger: true,
  };
}

/**
 * The mildest of these, and the only one undone by asking again. The
 * description says so, because a question with no consequence named reads as
 * more final than it is.
 */
export function removeFriend(t: Translator, subject: ConfirmSubject): ConfirmRequest {
  return {
    title: t("confirm.removeFriendTitle"),
    description: t("confirm.removeFriendDescription"),
    subject,
    confirmLabel: t("common.remove"),
    danger: true,
  };
}

export function blockPerson(t: Translator, subject: ConfirmSubject): ConfirmRequest {
  return {
    title: t("confirm.blockTitle"),
    description: t("confirm.blockDescription"),
    subject,
    confirmLabel: t("common.block"),
    danger: true,
  };
}

export function kickPerson(t: Translator, subject: ConfirmSubject): ConfirmRequest {
  return {
    title: t("confirm.kickTitle"),
    // A kicked person can return through an invite, which is the whole
    // difference from a ban.
    description: t("confirm.kickDescription"),
    subject,
    confirmLabel: t("common.kick"),
    busyLabel: t("confirm.kicking"),
    failure: t("confirm.kickFailed"),
    danger: true,
  };
}

export function kickFromChannelPerson(t: Translator, subject: ConfirmSubject): ConfirmRequest {
  return {
    title: t("confirm.kickChannelTitle"),
    description: t("confirm.kickChannelDescription"),
    subject,
    confirmLabel: t("voiceBar.disconnect"),
    busyLabel: t("confirm.disconnecting"),
    failure: t("confirm.kickChannelFailed"),
    danger: true,
  };
}

/** As long as a ban's reason may be. */
export const BAN_REASON_MAX_LENGTH = 200;

/**
 * The one that also asks for words. The bans list shows a fallback text when
 * no reason was given, so the field is offered but never required; a required
 * box would only teach people to type a full stop into it. A reason given
 * where the question was asked from, as with the composer's /ban, is already
 * in the field.
 */
export function banPerson(t: Translator, subject: ConfirmSubject, reason = ""): PromptRequest {
  const given = reason.trim().slice(0, BAN_REASON_MAX_LENGTH);
  return {
    title: t("confirm.banTitle"),
    description: t("confirm.banDescription"),
    subject,
    confirmLabel: t("common.ban"),
    busyLabel: t("confirm.banning"),
    failure: t("confirm.banFailed"),
    danger: true,
    field: {
      label: t("confirm.reasonLabel"),
      placeholder: t("confirm.reasonPlaceholder"),
      maxLength: BAN_REASON_MAX_LENGTH,
      optional: true,
      hint: t("confirm.banHint"),
      ...(given === "" ? {} : { initial: given }),
    },
  };
}

/**
 * Dropping a message that waits to go out. Nobody else knew it was coming,
 * so the only consequence to name is that its words go with it.
 */
export function deleteScheduled(t: Translator, subject: ConfirmSubject): ConfirmRequest {
  return {
    title: t("scheduled.deleteTitle"),
    description: t("scheduled.deleteDescription"),
    subject,
    confirmLabel: t("scheduled.delete"),
    busyLabel: t("confirm.deleting"),
    failure: t("scheduled.actionFailed"),
    danger: true,
  };
}

/** Deleting a channel, from its context menu or from its settings: the same card. */
export function deleteChannel(t: Translator, subject: ConfirmSubject): ConfirmRequest {
  return {
    title: t("confirm.deleteChannelTitle"),
    description: t("confirm.deleteChannelDescription"),
    subject,
    confirmLabel: t("channel.delete"),
    busyLabel: t("confirm.deleting"),
    failure: t("channel.deleteFailed"),
    danger: true,
  };
}

/**
 * Deleting a server. The name has to be typed: deleting cannot be undone, and
 * the server's name is the one thing nobody types by accident.
 */
export function deleteServer(t: Translator, subject: ConfirmSubject, name: string): PromptRequest {
  return {
    title: t("srv.delete.title"),
    description: t("srv.delete.detail"),
    subject,
    confirmLabel: t("srv.delete.button"),
    busyLabel: t("confirm.deleting"),
    failure: t("srv.delete.failed"),
    danger: true,
    field: { label: t("srv.delete.field"), match: name, maxLength: 80 },
  };
}

/** Deleting an empty section. */
export function deleteSection(t: Translator, subject: ConfirmSubject): ConfirmRequest {
  return {
    title: t("confirm.deleteSectionTitle"),
    description: t("section.deleteWarning"),
    subject,
    confirmLabel: t("section.delete"),
    busyLabel: t("confirm.deleting"),
    failure: t("section.deleteFailed"),
    danger: true,
  };
}

/**
 * A section that still holds channels cannot be deleted, so there is nothing
 * to confirm: the card says what to do first and has one answer.
 */
export function sectionNotEmpty(t: Translator, subject: ConfirmSubject): ConfirmRequest {
  return {
    title: t("confirm.sectionBlockedTitle"),
    description: t("confirm.sectionBlockedDetail"),
    subject,
    notice: true,
  };
}
