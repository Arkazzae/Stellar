import type { PersonVM } from "@/components/view-models";

/** What a seated person carries while being dragged to another voice channel. */
export interface VoiceDragData {
  type: "voice-occupant";
  person: PersonVM;
  sourceChannelId: string;
}

/** What a voice channel offers to whatever is dropped on it. */
export interface VoiceDropData {
  type: "voice-channel";
  channelId: string;
}

export function isVoiceDragData(value: unknown): value is VoiceDragData {
  if (typeof value !== "object" || value === null) return false;
  return (value as Partial<VoiceDragData>).type === "voice-occupant";
}

export function isVoiceDropData(value: unknown): value is VoiceDropData {
  if (typeof value !== "object" || value === null) return false;
  return (value as Partial<VoiceDropData>).type === "voice-channel";
}

/**
 * The move a drop asks for, or null when the pair is not an occupant over a
 * different voice channel.
 */
export function voiceMoveRequest(activeData: unknown, overData: unknown) {
  if (!isVoiceDragData(activeData) || !isVoiceDropData(overData)) return null;
  if (activeData.sourceChannelId === overData.channelId) return null;
  return {
    userId: activeData.person.id,
    displayName: activeData.person.displayName,
    sourceChannelId: activeData.sourceChannelId,
    targetChannelId: overData.channelId,
  };
}
