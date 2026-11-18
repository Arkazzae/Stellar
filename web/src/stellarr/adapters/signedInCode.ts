/**
 * The code only a signed-in person runs: the SignalR client and the WebRTC
 * voice client with what they bring in. Only `signedInLoader.ts` may import
 * this file, and only through a dynamic import; that is what keeps it out of
 * the files the sign-in pages wait for.
 */

import type { ApiClient } from "@/lib/api/client";
import type { RtcConfig } from "@/lib/api/types";
import { iceServers } from "@/lib/platform/env";
import { SignalRRealtimeClient } from "@/lib/realtime/signalr";
import type { RealtimeClient } from "@/lib/realtime/types";
import { RuntimeIceProvider } from "@/lib/voice/rtc/iceSession";
import { WebRtcVoiceClient } from "@/lib/voice/rtc/webrtc";
import { ParticipantPreferencesStore } from "@/lib/voice/participantPreferences";
import type { VoiceSettingsStore } from "@/lib/voice/settings";
import type { CreateVoiceOptions, VoicePort } from "@/ports";

export function createSignalRClient(): RealtimeClient {
  return new SignalRRealtimeClient();
}

export interface VoiceDependencies {
  api: ApiClient;
  realtime: RealtimeClient;
  /** The contract fetched at sign-in, absent until the boot payload is in. */
  rtcConfig: RtcConfig | undefined;
  settings: VoiceSettingsStore;
}

export function createWebRtcVoice(
  { api, realtime, rtcConfig, settings }: VoiceDependencies,
  { userId, displayName, translate }: CreateVoiceOptions,
): VoicePort {
  const ice = new RuntimeIceProvider({
    // TURN credentials are per user and expire, so each join asks for fresh
    // ones: one request per join, however many peers the channel has.
    fetchRtc: async () => (await api.bootstrap()).rtc,
    envServers: iceServers,
    isProduction: import.meta.env.PROD,
  });
  // The contract fetched at sign-in is the fallback when that refresh fails.
  ice.seed(rtcConfig?.iceServers);
  return new WebRtcVoiceClient({
    realtime,
    selfUserId: userId,
    selfDisplayName: displayName,
    translate,
    ice,
    // How each other person sounds to this account outlives the call and the
    // tab, in this browser only.
    participantPreferences: new ParticipantPreferencesStore(undefined, userId),
    // An instance without RTC is incompatible too, so a missing contract
    // becomes an explicit sentinel instead of an assumed v1.
    rtc: rtcConfig ?? { protocolVersion: 0 },
    getSettings: settings.getSnapshot,
    updateSettings: (patch) => settings.update(patch),
  });
}
