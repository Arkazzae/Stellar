import { describe, expect, it } from "vitest";
import {
  AUDIO_PROFILES,
  opusParameters,
  profileFor,
  usesWorkletSuppression,
  type AudioProfile,
} from "./audioProfiles";
import { AdaptiveBitrateController } from "./rtc/bitrate";
import { DEFAULT_SETTINGS } from "./settings";

describe("AUDIO_PROFILES", () => {
  it("keeps every adaptive range ordered and every ceiling above the profile before it", () => {
    const speech = ["economy", "balanced", "clarity", "premium", "studio"] as const;
    let previousCeiling = 0;
    for (const id of speech) {
      const profile = AUDIO_PROFILES[id];
      expect(profile.minBitrate).toBeLessThan(profile.maxBitrate);
      expect(profile.maxBitrate).toBeGreaterThan(previousCeiling);
      previousCeiling = profile.maxBitrate;
    }
  });

  it("uses DTX only for the bandwidth-saving profile", () => {
    const withDtx = Object.values(AUDIO_PROFILES).filter((profile) => profile.dtx);
    expect(withDtx.map((profile) => profile.id)).toEqual(["economy"]);
  });

  it("is mono speech everywhere except shared-screen music", () => {
    for (const profile of Object.values(AUDIO_PROFILES)) {
      expect(profile.stereo).toBe(profile.id === "music");
      expect(profile.contentHint).toBe(profile.id === "music" ? "music" : "speech");
    }
  });
});

const GOOD = { lossFraction: 0, rttMs: 40 };
const FAIR = { lossFraction: 0.04, rttMs: 120 };
const POOR = { lossFraction: 0.2, rttMs: 500 };

/** The controller the client builds for a profile, without smoothing lag. */
function controllerFor(profile: AudioProfile): AdaptiveBitrateController {
  return new AdaptiveBitrateController({
    ceiling: profile.maxBitrate,
    floor: profile.minBitrate,
    ...(profile.tiers === undefined ? {} : { tiers: profile.tiers }),
    smoothing: 1,
  });
}

/** Feeds one reading until the controller has had time to believe it. */
function settle(control: AdaptiveBitrateController, sample: typeof GOOD) {
  let decision = control.observe(sample);
  for (let i = 0; i < 5; i += 1) decision = control.observe(sample);
  return decision;
}

describe("the automatic quality", () => {
  it("lets the encoder go from Premium's ceiling down to the economy floor", () => {
    expect(AUDIO_PROFILES.auto).toMatchObject({
      maxBitrate: AUDIO_PROFILES.premium.maxBitrate,
      minBitrate: AUDIO_PROFILES.economy.minBitrate,
    });
    expect(AUDIO_PROFILES.auto).toMatchObject({ maxBitrate: 128_000, minBitrate: 16_000 });
  });

  it("steps down through the profiles: Balanced's ceiling on a fair link, Economy's on a poor one", () => {
    const control = controllerFor(AUDIO_PROFILES.auto);

    expect(settle(control, FAIR)).toMatchObject({ quality: "fair", bitrate: 64_000 });
    expect(settle(control, POOR)).toMatchObject({ quality: "poor", bitrate: 32_000 });
    expect(settle(control, GOOD)).toMatchObject({ quality: "good", bitrate: 128_000 });
  });

  it("leaves the fixed profiles on fractions of their own ceiling", () => {
    const control = controllerFor(AUDIO_PROFILES.premium);

    expect(settle(control, FAIR).bitrate).toBe(89_600);
    // 45% of Premium's ceiling is under its floor, so the floor holds.
    expect(settle(control, POOR).bitrate).toBe(64_000);

    // A switch carries the tiers with it, both ways, on a link still poor.
    const switchTo = ({ maxBitrate, minBitrate, tiers }: AudioProfile) =>
      control.setCeiling(maxBitrate, minBitrate, tiers);
    switchTo(AUDIO_PROFILES.auto);
    expect(control.getBitrate()).toBe(32_000);
    switchTo(AUDIO_PROFILES.clarity);
    expect(control.getBitrate()).toBe(43_200);
  });

  it("is speech like the other quality profiles: mono, 20 ms, FEC and no DTX", () => {
    expect(AUDIO_PROFILES.auto).toMatchObject({
      stereo: false,
      dtx: false,
      ptime: 20,
      contentHint: "speech",
    });
    expect(opusParameters(AUDIO_PROFILES.auto)).toEqual({
      useinbandfec: "1",
      usedtx: "0",
      maxaveragebitrate: "128000",
      stereo: "0",
      "sprop-stereo": "0",
      minptime: "10",
    });
  });
});

describe("profileFor", () => {
  it("resolves the profile named by the settings", () => {
    expect(profileFor({ ...DEFAULT_SETTINGS, opusProfile: "studio" })).toBe(
      AUDIO_PROFILES.studio,
    );
    expect(profileFor(DEFAULT_SETTINGS)).toBe(AUDIO_PROFILES.auto);
  });
});

describe("opusParameters", () => {
  it("asks for FEC, 20 ms packets and the profile's bitrate ceiling", () => {
    expect(opusParameters(AUDIO_PROFILES.premium)).toEqual({
      useinbandfec: "1",
      usedtx: "0",
      maxaveragebitrate: "128000",
      stereo: "0",
      "sprop-stereo": "0",
      minptime: "10",
    });
  });

  it("switches DTX on for the economy profile", () => {
    expect(opusParameters(AUDIO_PROFILES.economy)).toMatchObject({
      usedtx: "1",
      maxaveragebitrate: "32000",
    });
  });

  it("offers stereo for music and falls back to mono when the peer cannot take it", () => {
    expect(opusParameters(AUDIO_PROFILES.music)).toMatchObject({
      stereo: "1",
      "sprop-stereo": "1",
    });
    expect(opusParameters(AUDIO_PROFILES.music, false)).toMatchObject({
      stereo: "0",
      "sprop-stereo": "0",
    });
  });

  it("never turns a mono profile into stereo", () => {
    expect(opusParameters(AUDIO_PROFILES.studio, true)).toMatchObject({ stereo: "0" });
  });
});

describe("usesWorkletSuppression", () => {
  it("is true for the three local engines only", () => {
    expect(["rnnoise", "gtcrn", "speex"].every(usesWorkletSuppression)).toBe(true);
    expect(["browser", "off", "", "magic"].some(usesWorkletSuppression)).toBe(false);
  });
});
