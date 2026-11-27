import { useCallback, useMemo, useState, useSyncExternalStore } from "react";

import { banPerson, kickPerson, personSubject } from "@/components/confirmations";
import { useConfirm, usePrompt } from "@/components/primitives/confirmContext";
import { eventWindow } from "@/lib/events/eventSheetRequest";
import { draftScope } from "@/lib/platform/drafts";
import { PollSheet } from "@/components/room/poll/PollSheet";
import {
  Composer,
  type ComposerCommandHost,
  type ComposerScheduling,
} from "@/components/room/composer/Composer";
import { useGifsEnabled, useGiphyAccess } from "@/hooks/useGifsEnabled";
import type { Translator } from "@/i18n";
import { captureMicrophone, MicCaptureError } from "@/lib/voice/capture/micPipeline";
import { VoiceMessageCaptureError } from "@/ports";
import {
  useChatRoom,
  useDirectory,
  useEmoji,
  useLanguage,
  usePorts,
  useSession,
  useUi,
} from "@/state/hooks";

import { useMemberModeration } from "./useMemberMenu";
import { ScheduledDialogContainer } from "./ScheduledDialogContainer";
import { useReaderZone, useWaitingHere } from "./useScheduled";

interface RoomComposerProps {
  channelName: string;
  /** The thread being answered, or null for the channel itself. */
  threadRootId: string | null;
}

/**
 * The message field for the open channel, or for the open thread when there is
 * one. A thread is written to the thread, so it neither names a reply target
 * nor offers to cancel one.
 */
export function RoomComposer({ channelName, threadRootId }: RoomComposerProps) {
  const chat = useChatRoom();
  // The operator can turn GIFs off in Cockpit, and each person for themselves
  // in their settings; either way the composer offers no GIF button.
  const gifsOffered = useGifsEnabled();
  const giphyAccess = useGiphyAccess();
  const { members, server, canCreateEvents } = useDirectory();
  const { user } = useSession();
  const scope = user && chat.channelId ? draftScope(user.id, server.id, chat.channelId, threadRootId ?? "") : undefined;
  const { emoji: customEmoji } = useEmoji();
  const { locale, t } = useLanguage();
  const ports = usePorts();
  const ui = useUi();
  const moderation = useMemberModeration();
  const confirm = useConfirm();
  const prompt = usePrompt();

  // The device settings the call also runs on, so a recording uses the
  // microphone and processing the reader chose.
  const voiceSettings = useSyncExternalStore(
    ports.voiceSettings.subscribe,
    ports.voiceSettings.getSnapshot,
    ports.voiceSettings.getSnapshot,
  );

  const openVoiceMessageMicrophone = useCallback(async () => {
    try {
      // Push to talk belongs to calls. A recording is started and stopped by
      // its own button, so it is processed as in voice activity, or its gate
      // would stay shut with no key to open it.
      const capture = await captureMicrophone(
        { ...voiceSettings, inputMode: "voiceActivity" },
        { translate: t },
      );
      return {
        stream: capture.stream,
        inputTrack: capture.rawTrack,
        dispose: () => capture.dispose(),
      };
    } catch (cause) {
      throw new VoiceMessageCaptureError(captureFailure(cause, t));
    }
  }, [t, voiceSettings]);

  const inThread = threadRootId !== null;
  const reply = chat.replyingTo;

  // Sending later goes to the same place a send would: the channel, or the
  // thread written in. Its list opens over this place only.
  const place = chat.channelId === null
    ? null
    : { kind: "channel" as const, channelId: chat.channelId, ...(threadRootId === null ? {} : { threadRootId }) };
  const zone = useReaderZone();
  const waiting = useWaitingHere(place);
  const [listOpen, setListOpen] = useState(false);
  const [listNow, setListNow] = useState(() => new Date());
  // A new event is offered to be announced where it was asked for.
  const announceOn = chat.channelId;
  // A new poll's sheet, with the id its message goes out under: asking again
  // after an answer that was lost posts it once.
  const [pollSheet, setPollSheet] = useState<{ question: string; clientMessageId: string } | null>(null);
  const scheduling: ComposerScheduling | undefined =
    place === null
      ? undefined
      : {
          onSchedule: chat.schedule,
          waiting,
          place: inThread ? "thread" : "channel",
          onShowScheduled: () => {
            setListNow(new Date());
            setListOpen(true);
          },
          zone,
        };

  // As in the member menu, nobody kicks or bans themselves.
  const selfId = user?.id;
  const others = useMemo(
    () => members.filter((member) => member.id !== selfId),
    [members, selfId],
  );

  /*
   * Where the commands lead from a channel or a thread. The pinned list opens
   * as the header's button opens it, closing the thread it would sit beside.
   * Kicking and banning ask the same question cards the member menu does,
   * with the same deeds, and only for someone who may.
   */
  const commandHost: ComposerCommandHost = {
    openPinned: () => {
      chat.closeThread();
      if (ui.channelPanel !== "pinned") ui.toggleChannelPanel("pinned");
    },
    openStatus: ui.revealAccountPanel,
    people: others,
    ...(moderation.canKick
      ? {
          kick: (person) => {
            void confirm({
              ...kickPerson(t, personSubject(person)),
              run: async () => moderation.onKick(person.id),
            });
          },
        }
      : {}),
    ...(moderation.canBan
      ? {
          ban: (person, reason) => {
            void prompt({
              ...banPerson(t, personSubject(person), reason),
              run: async (answer) => moderation.onBan(person.id, answer),
            });
          },
        }
      : {}),
  };

  return (
    <>
      <Composer
        key={scope}
        draftScope={scope}
        channelName={channelName}
        recordingScope={
          inThread ? `${chat.channelId ?? "no-channel"}:thread:${threadRootId}` : chat.channelId
        }
        restoringFiles={chat.restoringFiles}
        chips={chat.chips}
        onSubmit={chat.send}
        openVoiceMessageMicrophone={openVoiceMessageMicrophone}
        onSendVoice={chat.sendVoice}
        onAttach={chat.attach}
        onRemoveChip={chat.removeChip}
        onRetryChip={chat.retryChip}
        onReorderChips={chat.reorderChips}
        onTyping={chat.notifyTyping}
        customEmoji={customEmoji}
        mentionCandidates={members}
        commandHost={commandHost}
        {...(scheduling === undefined ? {} : { scheduling })}
        {...(gifsOffered && giphyAccess === "on" ? { onGif: chat.sendGif } : {})}
        {...(chat.channelId === null
          ? {}
          : {
              onPoll: (question: string) => setPollSheet({ question, clientMessageId: crypto.randomUUID() }),
            })}
        {...(canCreateEvents && announceOn !== null
          ? { onEvent: () => eventWindow.open({ kind: "create", announceOn }) }
          : {})}
        locale={locale}
        t={t}
        {...(inThread
          ? { placeholder: t("room.threadPlaceholder"), replyingTo: null }
          : {
              replyingTo:
                reply === null
                  ? null
                  : { id: reply.id, authorName: reply.authorName, preview: reply.preview },
              onCancelReply: chat.cancelReply,
            })}
      />
      {listOpen && place !== null ? (
        <ScheduledDialogContainer place={place} now={listNow} zone={zone} onClose={() => setListOpen(false)} />
      ) : null}
      {pollSheet === null ? null : (
        <PollSheet
          initialQuestion={pollSheet.question}
          place={t(inThread ? "composer.poll.inThread" : "composer.poll.inChannel", { channel: channelName })}
          customEmoji={customEmoji}
          onSend={(draft) => chat.sendPoll(draft, pollSheet.clientMessageId)}
          onClose={() => setPollSheet(null)}
          t={t}
        />
      )}
    </>
  );
}

/** The sentence for why the microphone could not be opened. */
function captureFailure(cause: unknown, t: Translator): string {
  if (!(cause instanceof MicCaptureError)) return t("recorder.startFailed");
  switch (cause.kind) {
    case "denied":
      return t("recorder.allowMic");
    case "unsupported":
      return t("recorder.unsupported");
    default:
      return t("voice.microphoneNotWorking");
  }
}
