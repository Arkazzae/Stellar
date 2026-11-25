import { isRecord } from "@/lib/api/expect";

const STORAGE_KEY = "stellarr.voice-resume";
const MAX_AGE_MS = 2 * 60_000;

interface VoiceResume {
  userId: string;
  channelId: string;
  privateLabel: string | null;
  muted: boolean;
  deafened: boolean;
}

export function saveVoiceResume(call: VoiceResume | null): void {
  try {
    if (call === null) window.sessionStorage.removeItem(STORAGE_KEY);
    else window.sessionStorage.setItem(STORAGE_KEY, JSON.stringify({ ...call, savedAt: Date.now() }));
  } catch {
    // Voice still works when browser storage is unavailable.
  }
}

export function readVoiceResume(userId: string): VoiceResume | null {
  try {
    const navigation = performance.getEntriesByType("navigation")[0] as
      | PerformanceNavigationTiming
      | undefined;
    if (navigation?.type !== "reload") return null;
    const stored: unknown = JSON.parse(window.sessionStorage.getItem(STORAGE_KEY) ?? "null");
    if (
      isRecord(stored) &&
      stored.userId === userId &&
      typeof stored.savedAt === "number" &&
      Date.now() - stored.savedAt < MAX_AGE_MS &&
      typeof stored.channelId === "string" &&
      (stored.privateLabel === null || typeof stored.privateLabel === "string") &&
      typeof stored.muted === "boolean" &&
      typeof stored.deafened === "boolean"
    ) {
      return {
        userId,
        channelId: stored.channelId,
        privateLabel: stored.privateLabel,
        muted: stored.muted,
        deafened: stored.deafened,
      };
    }
  } catch {
    // A malformed or unavailable entry cannot be resumed.
  }
  return null;
}
