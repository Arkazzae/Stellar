import { useCallback, useSyncExternalStore } from "react";

import { draftScope } from "@/lib/platform/drafts";
import { Composer } from "@/components/room/composer/Composer";
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
} from "@/state/hooks";

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
  const { members, server } = useDirectory();
  const { user } = useSession();
  const scope = user && chat.channelId ? draftScope(user.id, server.id, chat.channelId, threadRootId ?? "") : undefined;
  const { emoji: customEmoji } = useEmoji();
  const { locale, t } = useLanguage();
  const ports = usePorts();

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

  return (
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
      {...(gifsOffered && giphyAccess === "on" ? { onGif: chat.sendGif } : {})}
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
