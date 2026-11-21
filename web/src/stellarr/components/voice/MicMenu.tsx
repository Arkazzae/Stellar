import { Icon } from "@stellarr/icons";
import { ProgressRing } from "@/components/primitives/ProgressRing";
import type { MessageKey, Translator } from "@/i18n";
import {
  NOISE_MODE_HINT,
  NOISE_MODE_LABEL,
  OPUS_PROFILE_HINT,
  OPUS_PROFILE_LABEL,
  opusCeilingKbps,
  type NoiseSuppressionMode,
  type OpusProfileId,
  type OpusQualityId,
  type SuppressorDownload,
  type VoiceSettings,
} from "@/ports";
import { useLanguage } from "@/state/LanguageProvider";

import { QualityMenu, type QualityGroup } from "./QualityMenu";

type ChangeSettings = (patch: Partial<VoiceSettings>) => void;

/** The automatic choice first, then the lightest profile to the heaviest, as in the settings. */
const QUALITY_ORDER: readonly OpusQualityId[] = [
  "auto",
  "economy",
  "balanced",
  "clarity",
  "premium",
  "studio",
];

/** What each fixed profile aims for, as the menu prints it. */
const PROFILE_BITRATE: Record<OpusProfileId, string> = {
  economy: "≈32 kb/s",
  balanced: "≈64 kb/s",
  clarity: "≈96 kb/s",
  premium: "128 kb/s",
  studio: "256 kb/s",
};

/** The settings' order: the browser's own suppressor first, raw sound last. */
const NOISE_ORDER: readonly NoiseSuppressionMode[] = [
  "browser",
  "rnnoise",
  "gtcrn",
  "speex",
  "off",
];

/** How the head names the mode after the quality, as in "Premium, Speex". */
const NOISE_SUMMARY: Record<NoiseSuppressionMode, MessageKey> = {
  browser: "voice.noise.activeBrowser",
  rnnoise: "voice.noise.rnnoise",
  gtcrn: "voice.noise.gtcrn",
  speex: "voice.noise.speex",
  off: "voice.noise.summaryOff",
};

/** The chosen engine's files on their way, in numbers and in words. */
export interface MicDownload {
  percent: number | null;
  text: string;
}

export interface MicMenuContent {
  summary: string;
  groups: QualityGroup[];
  downloading: MicDownload | null;
}

function qualityDetail(t: Translator, id: OpusQualityId): string {
  return id === "auto"
    ? t("voice.profile.autoDetail", { kbps: opusCeilingKbps(id) })
    : PROFILE_BITRATE[id];
}

/**
 * The microphone menu's content: the audio quality and the noise reduction,
 * each with a line about the one chosen, and a head naming both.
 *
 * A download is drawn only on the mode that is chosen now. The voice state
 * reports the mode the audio has settled on, which trails a fresh choice by
 * the settle delay, so a download of the mode left behind is not this menu's
 * to show.
 */
export function micMenu(
  settings: VoiceSettings,
  download: SuppressorDownload | null,
  t: Translator,
  change: ChangeSettings,
): MicMenuContent {
  const quality = settings.opusProfile;
  const noise = settings.noiseSuppression;
  const fetching = download?.kind === noise ? download : null;
  const downloading =
    fetching === null
      ? null
      : {
          percent: fetching.percent,
          text:
            fetching.percent === null
              ? t("voice.noise.downloadingUnknown", { name: t(NOISE_MODE_LABEL[noise]) })
              : t("voice.noise.downloading", {
                  name: t(NOISE_MODE_LABEL[noise]),
                  percent: fetching.percent,
                }),
        };

  return {
    summary: t("voiceBar.micSummary", {
      quality: t(OPUS_PROFILE_LABEL[quality]),
      noise: downloading?.text ?? t(NOISE_SUMMARY[noise]),
    }),
    groups: [
      {
        caption: t("voiceBar.audioQuality"),
        options: QUALITY_ORDER.map((id) => ({
          id,
          label: t(OPUS_PROFILE_LABEL[id]),
          detail: qualityDetail(t, id),
          selected: id === quality,
          onSelect: () => change({ opusProfile: id }),
        })),
        hint: t(OPUS_PROFILE_HINT[quality], { kbps: opusCeilingKbps(quality) }),
      },
      {
        caption: t("voice.noiseReduction"),
        options: NOISE_ORDER.map((id) => ({
          id,
          label: t(NOISE_MODE_LABEL[id]),
          selected: id === noise,
          ...(id === fetching?.kind ? { download: { percent: fetching.percent } } : {}),
          onSelect: () => change({ noiseSuppression: id }),
        })),
        hint: t(NOISE_MODE_HINT[noise]),
      },
    ],
    downloading,
  };
}

export interface MicMenuProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  settings: VoiceSettings;
  /** The chosen suppressor's files on their way, as the voice state reports them. */
  download: SuppressorDownload | null;
  onChangeSettings: ChangeSettings;
  /** The voice settings dialog, for everything the menu leaves out. */
  onMoreSettings: () => void;
  /** The arrow's own look: the bar's and the dock's differ in size. */
  caretClassName: string;
}

/**
 * The menu behind the microphone's arrow, in the call's bar and in the dock
 * alike: quality and noise reduction in one place.
 *
 * While the chosen suppressor downloads, the arrow carries the ring the dock's
 * noise button once had, so the download shows even with the menu closed.
 */
export function MicMenu({
  open,
  onOpenChange,
  settings,
  download,
  onChangeSettings,
  onMoreSettings,
  caretClassName,
}: MicMenuProps) {
  const { t } = useLanguage();
  const menu = micMenu(settings, download, t, onChangeSettings);
  const label = t("voiceBar.micQualityMenu");

  return (
    <QualityMenu
      open={open}
      onOpenChange={onOpenChange}
      className="mic-menu"
      title={t("voiceBar.micDrawerTitle")}
      summary={menu.summary}
      groups={menu.groups}
      footer={{ label: t("voiceBar.moreSoundSettings"), onSelect: onMoreSettings }}
      renderTrigger={(trigger) => (
        <button
          type="button"
          className={`${caretClassName} mic-caret`}
          aria-label={label}
          // The download in words, for a pointer and as the description.
          title={menu.downloading?.text ?? label}
          aria-busy={menu.downloading ? true : undefined}
          {...trigger}
        >
          {menu.downloading ? (
            <ProgressRing percent={menu.downloading.percent} size={16} className="caret-ring" />
          ) : null}
          <Icon name="caret" size={12} className="caret-icon" />
        </button>
      )}
    />
  );
}
