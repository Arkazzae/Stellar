import { documentLocale, translateForDocument } from "@/i18n/defaultLocale";
import { useMemo, useState, type MouseEvent, type ReactNode } from "react";

import { Icon } from "@stellarr/icons";
import { messageSubject } from "@/components/confirmations";
import { Avatar } from "@/components/primitives/Avatar";
import { useConfirm } from "@/components/primitives/ConfirmDialog";
import { ContextMenu, useContextMenu } from "@/components/primitives/ContextMenu";
import { identityVars } from "@/design/identity";
import { formatFull } from "@/lib/format/format";
import type {
  CustomEmoji,
  MessageReference,
  Poll,
  Reaction,
  ServerEvent,
  SealedReason,
  TimelineLine,
} from "@/ports";

import { Attachment } from "../attachments/Attachment";
import type { MessageTranslationOptions } from "../attachments/TranslatedTextAttachment";
import { plainText, type PointedAt } from "@/lib/messages/messageExport";
import { MessageEditor } from "./MessageEditor";
import { MessageEntry } from "./MessageEntry";
import { messageMenuEntries } from "./messageMenu";
import type { MessageAction } from "./MessageToolbar";
import { isJumbo, type AttachmentCallbacks, type MessagePart } from "@/lib/messages/parts";
import { rememberReaction } from "@/lib/emoji/quickReactions";
import { Reactions } from "./Reactions";
import { PollCard, type PollCallbacks } from "../poll/PollCard";

/** One message inside a block: its own id, and what it renders as. */
export interface MessageBodyVM {
  id: string;
  parts: MessagePart[];
  /** The plaintext of an end-to-end encrypted message: its translation stays in memory. */
  encrypted?: boolean;
  reactions?: Reaction[];
  pinned?: boolean;
  /** A question with answers to choose from; a message with one is never edited. */
  poll?: Poll;
  /** The event this message announces, as it was when the message was read. */
  event?: ServerEvent;
}

/**
 * A row that is not somebody's message: a line about the conversation, or
 * messages this browser cannot read. The transcript's owner draws it.
 */
export type TranscriptNotice =
  | { kind: "line"; line: TimelineLine; actorName: string }
  | { kind: "sealed"; reason: SealedReason; count: number };

/** What one author block on screen is. */
export interface MessageGroupVM {
  /** Stable identity for the block's row; the head message's id. */
  id: string;
  authorId: string;
  authorName: string;
  /**
   * The author's picture. Not on the message — `MessageAuthor` carries an id
   * and a name — so the provider resolves it from whoever knows the person,
   * and the initials stay underneath for everyone it cannot.
   */
  authorAvatarUrl?: string;
  /** A server notice: server identity, no profile actions, the same body grammar as any message. */
  system?: boolean;
  /**
   * You wrote this. Set by whoever builds the block, because only the provider
   * knows who is signed in — a component that compared ids itself would need
   * the session, and it may not have it.
   */
  own?: boolean;
  /** ISO 8601, of the first message in the block. */
  timestamp: string;
  /**
   * One entry per message, not one flat list of parts.
   *
   * A block is a run of messages from the same person within a few minutes, so
   * it can hold several — and each of them is separately editable, separately
   * deletable, and separately the thing a reader means when they say "that
   * one". Flattening them lost the only thing that could tell them apart.
   */
  bodies: MessageBodyVM[];
  /** The message this block answers; the line above it names the original. */
  replyTo?: MessageReference;
  /** Set when the block is a notice rather than messages; see `TranscriptNotice`. */
  notice?: TranscriptNotice;
}

export interface MessageGroupProps extends MessageGroupVM {
  eagerAvatar?: boolean;
  /** Jumps to the answered message; absent leaves the line inert. */
  onOpenReply?: (messageId: string) => void;
  onReact?(messageId: string, emoji: string): void;
  /** A deletion's promise keeps the question card open until it settles. */
  onAction?(messageId: string, action: MessageAction): Promise<void> | void;
  attachments?: AttachmentCallbacks;
  translation?: MessageTranslationOptions;
  /**
   * The message being rewritten right now, if it is one of this block's.
   *
   * Editing happens where the message is: the body is replaced by a field
   * holding its text, in the place the reader was already looking. The
   * alternative — sending them to the composer at the bottom of the screen —
   * moves the caret away from the sentence being changed.
   */
  editing?: { id: string; original: string } | null;
  onEditSave?(messageId: string, text: string): Promise<void> | void;
  onEditCancel?(): void;
  /** The server's own emoji, offered by this block's reaction picker. */
  customEmoji?: readonly CustomEmoji[];
  /** Votes and ends of this block's polls; without them a poll only shows. */
  polls?: PollCallbacks;
  /** Draws an announced event's card, kept current by whoever knows the events. */
  renderEvent?: (event: ServerEvent) => ReactNode;
  /** You may moderate here: the bin appears on other people's messages too. */
  canModerate?: boolean;
  /** You may add reactions here, which puts the quick ones in the toolbar. */
  canAddReactions?: boolean;
  /** Adds "Kopiuj identyfikator" to the right-click menu, as the preference
   *  in "Zaawansowane" promises it will. */
  developerMode?: boolean;
  /**
   * Raises the author's mini profile beside the block. Absent — and always so
   * for a system notice, which has no author to open — leaves the avatar and
   * the name as plain text.
   */
  onOpenAuthor?: (anchor: HTMLElement) => void;
  /**
   * True briefly after this block arrives, so its attachments can start with
   * eager priority. The row itself does not animate: the transcript already
   * moves to make room for it and a second vertical motion looks like a jump.
   */
  fresh?: boolean;
  /** Briefly marked after a jump landed on this block. */
  highlight?: boolean;
}

/** The index a data attribute on the nearest matching ancestor carries, if it is one. */
function indexFrom(origin: Element | null, selector: string, attribute: string): number | null {
  const value = Number(origin?.closest<HTMLElement>(selector)?.dataset[attribute]);
  return Number.isInteger(value) ? value : null;
}

function pointedAt(event: MouseEvent): PointedAt {
  const origin = event.target instanceof Element ? event.target : null;
  return {
    part: indexFrom(origin, "[data-message-part-index]", "messagePartIndex"),
    media: indexFrom(origin, "[data-media-index]", "mediaIndex"),
  };
}

/**
 * The answered message, one line above the block and half-height, with the
 * elbow drawn in CSS: a reference to the conversation, not a second copy of
 * it, so it never wraps and never grows.
 */
function ReplyLine({
  reference,
  goToLabel,
  onOpen,
}: {
  reference: MessageReference;
  goToLabel: string;
  onOpen?: (messageId: string) => void;
}) {
  const content = (
    <>
      <b>{`@${reference.authorName}`}</b>
      <i>{reference.preview}</i>
    </>
  );
  return (
    <div className="reply-line">
      <span className="reply-elbow" aria-hidden="true" />
      {onOpen === undefined ? (
        <span className="reply-ref">{content}</span>
      ) : (
        <button
          type="button"
          className="reply-ref"
          title={goToLabel}
          onClick={() => onOpen(reference.id)}
        >
          {content}
        </button>
      )}
    </div>
  );
}

export function MessageGroup({
  authorId,
  authorName,
  authorAvatarUrl,
  eagerAvatar,
  system = false,
  own = false,
  timestamp,
  bodies,
  replyTo,
  onOpenReply,
  onReact,
  onAction,
  attachments,
  translation,
  editing = null,
  onEditSave,
  onEditCancel,
  developerMode = false,
  customEmoji,
  polls,
  renderEvent,
  canModerate = false,
  canAddReactions = true,
  onOpenAuthor,
  fresh = false,
  highlight = false,
}: MessageGroupProps) {
  const confirm = useConfirm();
  const menu = useContextMenu();
  const [menuTarget, setMenuTarget] = useState<{ bodyId: string; pointed: PointedAt } | null>(
    null,
  );
  const locale = translation?.locale ?? documentLocale();
  const t = translation?.t ?? translateForDocument;
  // The same settings, held in memory only, for the plaintext of an encrypted message.
  const privateTranslation = useMemo(
    () => (translation === undefined ? undefined : { ...translation, persist: false }),
    [translation],
  );

  // A notice from the server is not a person, so nothing about it opens a
  // profile — the server avatar and name stay inert.
  const openAuthor = system ? undefined : onOpenAuthor;
  const menuBody = bodies.find((body) => body.id === menuTarget?.bodyId);

  const closeMenu = () => {
    menu.close();
    setMenuTarget(null);
  };

  // A chip, the toolbar's quick row and its picker all toggle through here.
  // One that adds becomes a recent, as a pick in the picker always does.
  const toggleOn = (body: MessageBodyVM) =>
    onReact &&
    ((emoji: string) => {
      const mine = body.reactions?.some((item) => item.emoji === emoji && item.mine) ?? false;
      if (!mine) rememberReaction(emoji);
      onReact(body.id, emoji);
    });

  // The message as the question card shows it; called only when it asks.
  const describe = (parts: readonly MessagePart[], poll?: Poll) => () =>
    messageSubject(t, {
      authorId,
      authorName,
      authorAvatarUrl,
      // A poll with no text of its own is known by its question.
      text: plainText(parts) || (poll?.question ?? ""),
    });

  return (
    <div
      className={highlight ? "msg-group jumped" : "msg-group"}
      // The person's hue is derived once and published as custom properties, so
      // the avatar and the name stay in sync without either hard-coding a value.
      style={system ? undefined : identityVars(authorId)}
    >
      {menu.at === null || menuTarget === null || menuBody === undefined ? null : (
        <ContextMenu
          at={menu.at}
          entries={messageMenuEntries({
            messageId: menuBody.id,
            parts: menuBody.parts,
            pinned: menuBody.pinned === true,
            pointed: menuTarget.pointed,
            own,
            fixed: menuBody.poll !== undefined,
            canModerate,
            developerMode,
            onAction: onAction && ((action) => onAction(menuBody.id, action)),
            describe: describe(menuBody.parts, menuBody.poll),
            ...(attachments?.resolveOriginal === undefined ? {} : { resolveOriginal: attachments.resolveOriginal }),
            confirm,
            t,
          })}
          label={t("message.contextLabel")}
          onClose={closeMenu}
        />
      )}
      {/* The server may send an explicit null for a message that answers nothing. */}
      {replyTo === undefined || replyTo === null ? null : (
        <ReplyLine reference={replyTo} goToLabel={t("message.goTo")} onOpen={onOpenReply} />
      )}
      {/* A control when there is a profile behind it: the name beside it opens
          the same card, but a face is what a pointer aims at, and it has to be
          reachable by keyboard too. */}
      <Avatar
        id={authorId}
        displayName={authorName}
        avatarUrl={authorAvatarUrl}
        eager={eagerAvatar}
        size="md"
        system={system}
        onOpen={openAuthor}
      />
      <div className="g-main">
        <div className="msg-head">
          {openAuthor === undefined ? (
            <b
              className={system ? "sys-name" : undefined}
              style={system ? undefined : { color: "var(--person-name)" }}
            >
              {authorName}
            </b>
          ) : (
            <button
              type="button"
              className="who-open"
              style={{ color: "var(--person-name)" }}
              aria-haspopup="dialog"
              onClick={(event) => openAuthor(event.currentTarget)}
            >
              {authorName}
            </button>
          )}
          {bodies[0]?.event === undefined ? null : <span className="msg-tag">{t("event.tag")}</span>}
          <time dateTime={timestamp}>{formatFull(timestamp, locale)}</time>
        </div>
        {bodies.map((body) => {
          // "🎉🎉" is a cheer, not a sentence, so it is drawn at the size a cheer
          // deserves. A notice is never one, whatever it contains.
          const bodyClass = !system && isJumbo(body.parts) ? "msg-body jumbo" : "msg-body";
          return (
            <MessageEntry
              key={body.id}
              onContextMenu={
                system
                  ? undefined
                  : (event) => {
                      setMenuTarget({ bodyId: body.id, pointed: pointedAt(event) });
                      menu.onContextMenu(event);
                    }
              }
              actions={{
                onReact: toggleOn(body),
                onAction: onAction && ((action) => onAction(body.id, action)),
                describe: describe(body.parts, body.poll),
                customEmoji,
                reactions: body.reactions,
                // A server notice is offered no quick reactions.
                canAddReactions: canAddReactions && !system,
                // A server notice belongs to nobody and is edited by nobody;
                // moderating one would delete the record of what the server did.
                own: own && !system,
                fixed: body.poll !== undefined,
                canModerate: canModerate && !system,
                pinned: body.pinned,
                t,
              }}
            >
              {body.pinned === true ? (
                <span className="msg-pinned">
                  <Icon name="pin" size={12} />
                  {t("message.pinned")}
                </span>
              ) : null}
              {editing?.id === body.id ? (
                <MessageEditor
                  original={editing.original}
                  onSave={(text) => onEditSave?.(body.id, text)}
                  onCancel={() => onEditCancel?.()}
                  t={t}
                />
              ) : (
                body.parts.map((part, index) => (
                  <div className={bodyClass} data-message-part-index={index} key={index}>
                    <Attachment
                      part={part}
                      callbacks={attachments}
                      priority={fresh}
                      translation={body.encrypted === true ? privateTranslation : translation}
                    />
                  </div>
                ))
              )}
              {body.poll === undefined ? null : (
                <PollCard
                  messageId={body.id}
                  poll={body.poll}
                  canEnd={own || canModerate}
                  {...(polls === undefined ? {} : { actions: polls })}
                  customEmoji={customEmoji}
                />
              )}
              {body.event === undefined || renderEvent === undefined ? null : renderEvent(body.event)}
              {body.reactions === undefined || body.reactions.length === 0 ? null : (
                <Reactions
                  reactions={body.reactions}
                  onToggle={toggleOn(body)}
                  customEmoji={customEmoji}
                />
              )}
            </MessageEntry>
          );
        })}
      </div>
    </div>
  );
}
