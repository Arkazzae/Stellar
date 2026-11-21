import type { OpusProfileId } from "./settings";

/**
 * The ceiling each voice profile hands the encoder, in kilobits a second.
 *
 * A module of its own so both the encoder (`audioProfiles`) and the settings
 * can read it: the settings already import the encoder's module, and a value
 * passed the other way would be read before it exists.
 */
export const OPUS_PROFILE_KBPS: Record<OpusProfileId, number> = {
  economy: 32,
  balanced: 64,
  clarity: 96,
  premium: 128,
  studio: 256,
};

/**
 * The profile whose ceiling the automatic quality starts from. The encoder's
 * bound, the "up to" in the menu and the hint all read this one value, so
 * raising it to Studio is a change of this line alone.
 */
export const AUTO_CEILING_PROFILE: OpusProfileId = "premium";
