import { AUTO_CEILING_PROFILE, OPUS_PROFILE_KBPS } from "./opusRates";
import type { BitrateTiers } from "./rtc/bitrate";
import type { NoiseSuppressionMode, OpusQualityId, VoiceSettings } from "./settings";

/**
 * Opus is configured, never replaced.
 *
 * WebRTC mandates Opus (RFC 7874), and the only supported tuning surfaces are
 * fmtp parameters in the SDP and `RTCRtpSender.setParameters`. Every profile
 * below expresses its target through those two:
 *
 * - `useinbandfec=1`: RFC 8854 recommends Opus's built-in FEC for WebRTC. It
 *   repairs a lost packet from the next one, the biggest quality win on a home
 *   uplink.
 * - `usedtx=0` for quality speech: continuous transmission leaves Opus no onset
 *   decision to get wrong. Only the bandwidth-saving profile uses DTX. The
 *   application gate (see `micPipeline`) works in the capture graph with
 *   lookahead, so Opus still sees a continuous stream.
 * - `stereo=0` for speech: one voice, one channel; the bits go to quality.
 * - `minptime=10` with `a=ptime:20`: 20 ms packets, the Opus default and the
 *   best latency/overhead trade for speech.
 */
export type AudioProfileId = OpusQualityId | "music";

export interface AudioProfile {
  id: AudioProfileId;
  /** Ceiling handed to `setParameters` and to `maxaveragebitrate`. */
  maxBitrate: number;
  /** Floor the adaptive controller may never go below. */
  minBitrate: number;
  stereo: boolean;
  dtx: boolean;
  ptime: number;
  contentHint: "speech" | "music";
  /**
   * What the adaptive controller aims for on each measured tier. Without it
   * the controller takes fractions of `maxBitrate`.
   */
  tiers?: BitrateTiers;
}

function speechProfile(
  id: OpusQualityId,
  maxBitrate: number,
  minBitrate: number,
  dtx = false,
): AudioProfile {
  return { id, maxBitrate, minBitrate, stereo: false, dtx, ptime: 20, contentHint: "speech" };
}

/** The lowest any speech profile goes: the economy profile's floor. */
const ECONOMY_FLOOR = 16_000;

const AUTO_CEILING = OPUS_PROFILE_KBPS[AUTO_CEILING_PROFILE] * 1000;

export const AUDIO_PROFILES: Record<AudioProfileId, AudioProfile> = {
  // One range across the profiles, from the ceiling of `AUTO_CEILING_PROFILE`
  // down to economy's floor. Rather than fractions of its own ceiling, it steps
  // down through the profiles: their ceiling on a good link, balanced's on a
  // fair one and economy's on a poor one. DTX stays off, as in the other
  // quality profiles.
  auto: {
    ...speechProfile("auto", AUTO_CEILING, ECONOMY_FLOOR),
    tiers: {
      good: AUTO_CEILING,
      fair: OPUS_PROFILE_KBPS.balanced * 1000,
      poor: OPUS_PROFILE_KBPS.economy * 1000,
    },
  },
  economy: speechProfile("economy", OPUS_PROFILE_KBPS.economy * 1000, ECONOMY_FLOOR, true),
  balanced: speechProfile("balanced", OPUS_PROFILE_KBPS.balanced * 1000, 24_000),
  clarity: speechProfile("clarity", OPUS_PROFILE_KBPS.clarity * 1000, 32_000),
  premium: speechProfile("premium", OPUS_PROFILE_KBPS.premium * 1000, 64_000),
  // Opus can spend roughly 256 kb/s on one full-band mono channel. Above that a
  // voice microphone gains no useful detail, and stereo would only encode room
  // position while doubling what every listener has to receive.
  studio: speechProfile("studio", OPUS_PROFILE_KBPS.studio * 1000, 96_000),
  // Shared screen audio is music, a video soundtrack or a game, not speech. DTX
  // would clip the quiet passages it is meant to carry, and stereo is part of
  // the content rather than a luxury.
  music: {
    id: "music",
    maxBitrate: 160_000,
    minBitrate: 96_000,
    stereo: true,
    dtx: false,
    ptime: 20,
    contentHint: "music",
  },
};

export function profileFor(settings: VoiceSettings): AudioProfile {
  return AUDIO_PROFILES[settings.opusProfile];
}

/**
 * fmtp parameters for a profile, ready to merge into an audio m-section.
 * `stereoSupported` is what the remote description allows; a stereo profile
 * falls back to mono when the peer did not offer stereo.
 */
export function opusParameters(
  profile: AudioProfile,
  stereoSupported = profile.stereo,
): Record<string, string> {
  const stereo = profile.stereo && stereoSupported ? "1" : "0";
  return {
    useinbandfec: "1",
    usedtx: profile.dtx ? "1" : "0",
    maxaveragebitrate: String(profile.maxBitrate),
    stereo,
    "sprop-stereo": stereo,
    minptime: "10",
  };
}

/** The suppressors that run as an AudioWorklet in our own capture graph. */
export type WorkletSuppressor = Extract<NoiseSuppressionMode, "rnnoise" | "gtcrn" | "speex">;

/** True when the pipeline has to insert a suppressor worklet of its own. */
export function usesWorkletSuppression(
  mode: NoiseSuppressionMode | string,
): mode is WorkletSuppressor {
  return mode === "rnnoise" || mode === "gtcrn" || mode === "speex";
}
