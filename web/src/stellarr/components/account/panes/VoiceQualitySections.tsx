import { useId, type CSSProperties } from "react";

import { Switch } from "@/components/primitives/Switch";
import {
  AUTO_CEILING_PROFILE,
  OPUS_PROFILE_HINT,
  OPUS_PROFILE_KBPS,
  OPUS_PROFILE_LABEL,
  opusCeilingKbps,
  type OpusProfileId,
  type VoiceSettings,
} from "@/ports";
import { useLanguage } from "@/state/hooks";

import { FormRow, RangeField } from "./VoiceFields";

interface QualityRowProps {
  settings: VoiceSettings;
  onChange: (patch: Partial<VoiceSettings>) => void;
}

/** From what a poor link can carry to what a studio would keep. */
const OPUS_ORDER: readonly OpusProfileId[] = [
  "economy",
  "balanced",
  "clarity",
  "premium",
  "studio",
];

/**
 * The Opus bitrate profile, the last row of the "your voice" group: what the
 * voice costs on the connection.
 *
 * One slider with five stops rather than five cards: the choice is a degree,
 * and each stop names its bitrate under the track. It belongs to the person
 * rather than the channel for now: the server keeps no channel-level audio
 * settings, so this is the only choice that reaches the encoder.
 *
 * The automatic choice comes first, as in the call's microphone menu: a switch
 * over the slider, on by default. While it is on the encoder follows the link
 * on its own, so the slider rests on the stop it starts from and takes no
 * input; switching it off keeps that stop.
 *
 * The screen-share quality is not here. It is chosen in the call's own bar,
 * while sharing, where its effect can be seen.
 */
export function VoiceAudioQualityRow({ settings, onChange }: QualityRowProps) {
  const { t } = useLanguage();
  const id = useId();
  const quality = settings.opusProfile;
  const automatic = quality === "auto";
  const profile = automatic ? AUTO_CEILING_PROFILE : quality;
  const kbps = opusCeilingKbps(quality);
  const autoName = t(OPUS_PROFILE_LABEL.auto);
  const reading = automatic ? t("voice.profile.autoDetail", { kbps }) : t(OPUS_PROFILE_LABEL[profile]);

  return (
    <FormRow label={t("voice.audioQuality")} detail={t("voice.audioQualityNote")} htmlFor={id}>
      <label className="vq-auto">
        <span>{autoName}</span>
        <Switch
          checked={automatic}
          label={autoName}
          onChange={(on) => onChange({ opusProfile: on ? "auto" : AUTO_CEILING_PROFILE })}
        />
      </label>
      <RangeField
        id={id}
        min={0}
        max={OPUS_ORDER.length - 1}
        step={1}
        value={OPUS_ORDER.indexOf(profile)}
        display={reading}
        valueText={automatic ? `${autoName}, ${reading}` : `${reading}, ${kbps} kb/s`}
        disabled={automatic}
        onChange={(index) => onChange({ opusProfile: OPUS_ORDER[index] ?? profile })}
      />
      <span
        className="vc-ticks"
        aria-hidden="true"
        style={{ "--stops": OPUS_ORDER.length - 1 } as CSSProperties}
      >
        {OPUS_ORDER.map((stop, index) => (
          <i key={stop} style={{ "--k": index } as CSSProperties}>
            {OPUS_PROFILE_KBPS[stop]}
          </i>
        ))}
      </span>
      <p className="ladder-note">{t(OPUS_PROFILE_HINT[quality], { kbps })}</p>
    </FormRow>
  );
}
