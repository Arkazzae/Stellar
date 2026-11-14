import { useState } from "react";

import { voiceMoveRequest, type VoiceDragData } from "@/components/shell/voiceMove";
import { useLanguage } from "@/state/hooks";

export interface VoiceMoveOptions {
  /** Absent leaves occupants as plain rows: nothing can be dropped anywhere. */
  onMove?: (
    userId: string,
    sourceChannelId: string,
    targetChannelId: string,
  ) => Promise<void> | void;
  /** Called with the failure text, or null when a new attempt clears it. */
  onError: (message: string | null) => void;
}

/**
 * The life of one staff move of a person between voice channels once they are
 * dropped: who is waiting for the server to confirm, and what a screen reader
 * hears while they wait. The drag itself belongs to each rail list; this is
 * shared by all of them, since a move outlives the list it started in.
 */
export function useVoiceMove({ onMove, onError }: VoiceMoveOptions) {
  const { t } = useLanguage();
  const [movingUserId, setMovingUserId] = useState<string | null>(null);
  const [announcement, setAnnouncement] = useState("");

  /** A release over `targetChannelId`, or over no channel at all. */
  const onDrop = (dragged: VoiceDragData, targetChannelId: string | null) => {
    const move = voiceMoveRequest(
      dragged,
      targetChannelId === null
        ? undefined
        : { type: "voice-channel", channelId: targetChannelId },
    );
    if (move === null || onMove === undefined) return;

    setMovingUserId(move.userId);
    onError(null);
    setAnnouncement(t("shell.movingUser", { name: move.displayName }));
    void Promise.resolve(
      onMove(move.userId, move.sourceChannelId, move.targetChannelId),
    )
      .then(() => {
        setAnnouncement(t("shell.movedUser", { name: move.displayName }));
      })
      .catch(() => {
        const failed = t("shell.moveFailed", { name: move.displayName });
        onError(failed);
        setAnnouncement(failed);
      })
      .finally(() => setMovingUserId(null));
  };

  return { movingUserId, announcement, onDrop };
}

export type VoiceMove = ReturnType<typeof useVoiceMove>;
