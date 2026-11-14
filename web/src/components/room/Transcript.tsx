"use client";

import { translateForDocument } from "@/i18n/defaultLocale";
import { useMemo, type ReactNode } from "react";

import { Icon } from "@stellarr/ui/icons";
import type { OpenProfile } from "@/components/view-models";
import type { CustomEmoji } from "@/ports";

import { MessageGroup } from "./MessageGroup";
import type { MessageAction } from "./MessageToolbar";
import type { MessageTranslationOptions } from "./attachments/TranslatedTextAttachment";
import type { AttachmentCallbacks } from "./parts";
import {
  messageTail,
  oldestGroupId,
  toRows,
  type TranscriptSectionVM,
} from "./transcriptRows";
import { useArrivals } from "./useArrivals";
import { useJumpTo } from "./useJumpTo";
import { useOlderHistory } from "./useOlderHistory";
import { useTranscriptScroll } from "./useTranscriptScroll";

export type { TranscriptSectionVM } from "./transcriptRows";

export interface TranscriptProps {
  sections: TranscriptSectionVM[];
  /** Region name, e.g. "Messages in #general". */
  label: string;
  /**
   * A thread already supplies the reply context around the whole transcript,
   * so repeating the root reference above every answer would make it read
   * like a stack of quoted replies instead of one conversation.
   */
  hideReplyReferences?: boolean;
  /** The first history page has not arrived yet. */
  loading?: boolean;
  translation?: MessageTranslationOptions;
  onReact?(messageId: string, emoji: string): void;
  onAction?(messageId: string, action: MessageAction): void;
  attachments?: AttachmentCallbacks;
  /** The server's own emoji, for the reaction picker on every block. */
  customEmoji?: readonly CustomEmoji[];
  /** You may moderate this channel: the bin appears on everyone's messages. */
  canModerate?: boolean;
  /** Adds "Kopiuj identyfikator" to every message's right-click menu. */
  developerMode?: boolean;
  /** The message being rewritten, wherever in the transcript it sits. */
  editing?: { id: string; original: string } | null;
  onEditSave?(messageId: string, text: string): Promise<void> | void;
  onEditCancel?(): void;
  /**
   * Changes when the reader sends something of their own.
   *
   * Following an arrival is conditional on already being at the end, which is
   * the right restraint for someone else's message — being yanked mid-read is
   * worse than missing a line. Your own message is not that: you performed an
   * action and the transcript owes you its result, wherever you were reading.
   */
  followOwnSend?: number;
  /** Raises a mini profile from an author's avatar or name. */
  onOpenAuthor?: OpenProfile;
  /**
   * Rendered above the first day separator, inside the scroller.
   *
   * A conversation begins somewhere and says so; a channel that has been going
   * for a year does not. Rendering it inside the scroller rather than as a
   * sibling is what makes it scroll away with the history instead of sitting
   * on top of it like a banner.
   */
  intro?: ReactNode;
  /**
   * The first block that arrived after this channel was last read. A line goes
   * above it, and stays there while the reader works down to it — the mark
   * behind this only moves when the channel is left.
   */
  firstUnreadGroupId?: string | null;
  /** Loads the next backwards page when the reader reaches the oldest row. */
  onLoadOlder?(): void;
  hasOlder?: boolean;
  loadingOlder?: boolean;
  /**
   * Takes the transcript's "go to this message" for whoever is outside it.
   *
   * Only this component knows where its rows are, so only it can perform the
   * scroll — but the channel panel's pinned list needs it. It is the same
   * function the reply line uses, so both doors behave identically: a message
   * older than the loaded history is not scrolled to, because there is nothing
   * to scroll to yet.
   */
  onJumpReady?(jumpTo: (messageId: string) => void): void;
}

/**
 * The message list.
 *
 * Every loaded row is rendered into one plain scroller; nothing is windowed.
 * The scroll behaviour a reader relies on is explicit instead:
 * `useTranscriptScroll` opens at the end, follows the end, and keeps the row
 * being read in place when anything above it changes height, and
 * `useOlderHistory` asks for the page before the oldest one as the reader
 * nears the top.
 */
export function Transcript({ loading = false, ...props }: TranscriptProps) {
  // The opening position is taken once, when the list mounts with its first
  // page. Until that page is here, the empty log keeps the pane's geometry and
  // tells assistive tech it is still busy.
  if (loading) {
    return (
      <div
        className="transcript"
        role="log"
        aria-live="polite"
        aria-label={props.label}
        aria-busy="true"
      />
    );
  }
  return <ReadyTranscript {...props} />;
}

function ReadyTranscript({
  sections,
  label,
  hideReplyReferences = false,
  translation,
  onReact,
  onAction,
  attachments,
  customEmoji,
  canModerate,
  developerMode,
  editing,
  onEditSave,
  onEditCancel,
  onOpenAuthor,
  intro,
  firstUnreadGroupId = null,
  followOwnSend,
  onLoadOlder,
  hasOlder = false,
  loadingOlder = false,
  onJumpReady,
}: Omit<TranscriptProps, "loading">) {
  const t = translation?.t ?? translateForDocument;
  const rows = useMemo(
    () => toRows(sections, firstUnreadGroupId),
    [sections, firstUnreadGroupId],
  );
  const tail = useMemo(() => messageTail(rows), [rows]);

  const { scrollerRef, contentRef, atBottom, toEnd, reveal } = useTranscriptScroll({
    rows,
    tail,
    followOwnSend,
  });
  useOlderHistory({
    scroller: scrollerRef,
    oldestId: oldestGroupId(rows),
    hasOlder,
    loadingOlder,
    onLoadOlder,
  });
  const { flashId, jumpTo } = useJumpTo(rows, reveal, onJumpReady);
  const { freshId, missed, dismissMissed } = useArrivals(rows.at(-1)?.id, atBottom);

  /*
   * The way back down belongs to being scrolled up, not to having missed
   * something: scrolling up is the only way to read anything older than a
   * screen, and the way back used to be the same drag in reverse. What
   * arrivals change is what it says — an arrow while reading, "New
   * messages" once something landed below, because the list deliberately
   * does not follow the end for a reader who is elsewhere.
   */
  const catchUp = () => {
    dismissMissed();
    toEnd();
  };

  return (
    // The scroller keeps its own class and its own box; this only gives the
    // catch-up button something to be positioned against.
    <div className="transcript-area">
      <div
        ref={scrollerRef}
        className="transcript"
        role="log"
        // Scrollable by keyboard once focused, as a log a reader moves through.
        tabIndex={0}
        aria-busy={false}
        aria-live="polite"
        aria-relevant="additions"
        aria-label={label}
      >
        {/* One box at the top of the column, whatever the intro is made of. */}
        {intro === undefined ? null : <div>{intro}</div>}
        <div ref={contentRef} className="transcript-inner">
          {rows.map((row) => (
            // One element per row, in the same order as the rows: the scroll
            // keeper finds a row's box by its position among these.
            <div key={row.id} data-row-id={row.id}>
              {row.kind === "day" ? (
                <div className="day-sep">{row.day}</div>
              ) : row.kind === "unread" ? (
                <div className="unread-sep">{t("message.unread")}</div>
              ) : (
                <MessageGroup
                  {...row.group}
                  replyTo={hideReplyReferences ? undefined : row.group.replyTo}
                  fresh={row.id === freshId}
                  highlight={row.id === flashId}
                  onOpenReply={hideReplyReferences ? undefined : jumpTo}
                  translation={translation}
                  attachments={attachments}
                  customEmoji={customEmoji}
                  canModerate={canModerate}
                  developerMode={developerMode}
                  editing={editing}
                  onEditSave={onEditSave}
                  onEditCancel={onEditCancel}
                  onReact={onReact}
                  onAction={onAction}
                  onOpenAuthor={
                    onOpenAuthor
                      ? (anchor) => onOpenAuthor(row.group.authorId, anchor)
                      : undefined
                  }
                />
              )}
            </div>
          ))}
        </div>
      </div>
      {atBottom ? null : (
        <button
          type="button"
          className={missed ? "tr-jump" : "tr-jump bare"}
          // The pill names itself; the bare arrow has to be named here.
          aria-label={missed ? undefined : t("message.goLatest")}
          onClick={catchUp}
        >
          {missed ? t("message.unread") : null}
          <Icon name="caret" size={missed ? 13 : 15} />
        </button>
      )}
    </div>
  );
}
