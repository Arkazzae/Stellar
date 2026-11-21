import type { MessageKey, Translator } from "@/i18n";
import { CAMERA_PROFILES, SCREEN_SHARE_PROFILES } from "@/ports";
import type { CameraQualityId, ScreenShareQualityId, VoiceSettings } from "@/ports";

import type { QualityGroup } from "./QualityMenu";

/**
 * What the camera's and the screen's arrow drawers show. The microphone's has
 * a menu of its own, `MicMenu`, which the dock opens too.
 */
interface QualityMenuContent {
  summary: string;
  groups: QualityGroup[];
}

type ChangeSettings = (patch: Partial<VoiceSettings>) => void;

// Every list below runs from the lightest profile to the heaviest.
const CAMERA_ORDER: readonly CameraQualityId[] = ["economy", "balanced", "sharp"];
const CAMERA_LABEL: Record<CameraQualityId, MessageKey> = {
  economy: "voice.cameraQuality.economy",
  balanced: "voice.cameraQuality.balanced",
  sharp: "voice.cameraQuality.sharp",
};

const SCREEN_ORDER: readonly ScreenShareQualityId[] = [
  "economy",
  "balanced",
  "sharp",
  "ultra",
];
const SCREEN_LABEL: Record<ScreenShareQualityId, MessageKey> = {
  economy: "voice.screen.economy",
  balanced: "voice.screen.balanced",
  sharp: "voice.screen.sharp",
  ultra: "voice.screen.ultra",
};

function choiceGroup<Id extends string>(
  caption: string,
  ids: readonly Id[],
  current: Id,
  describe: (id: Id) => { label: string; detail: string },
  choose: (id: Id) => void,
): QualityGroup {
  return {
    caption,
    options: ids.map((id) => ({
      id,
      ...describe(id),
      selected: id === current,
      onSelect: () => choose(id),
    })),
  };
}

function videoDetail(t: Translator, height: number, framerate: number): string {
  return t("voiceBar.videoDetail", { resolution: `${height}p`, framerate });
}

/** Without camera support there is nothing to choose, so the drawer only explains. */
export function cameraQualityMenu(
  settings: VoiceSettings,
  supported: boolean,
  t: Translator,
  change: ChangeSettings,
): QualityMenuContent {
  if (!supported) {
    return { summary: t("voiceBar.cameraUnavailable"), groups: [] };
  }
  const detailOf = (id: CameraQualityId) =>
    videoDetail(t, CAMERA_PROFILES[id].height, CAMERA_PROFILES[id].maxFramerate);
  const current = settings.cameraQuality;
  return {
    summary: t("voiceBar.cameraSummary", {
      quality: t(CAMERA_LABEL[current]),
      detail: detailOf(current),
    }),
    groups: [
      choiceGroup(
        t("voiceBar.cameraQuality"),
        CAMERA_ORDER,
        current,
        (id) => ({ label: t(CAMERA_LABEL[id]), detail: detailOf(id) }),
        (cameraQuality) => change({ cameraQuality }),
      ),
    ],
  };
}

export function screenQualityMenu(
  settings: VoiceSettings,
  t: Translator,
  change: ChangeSettings,
): QualityMenuContent {
  const detailOf = (id: ScreenShareQualityId) =>
    videoDetail(
      t,
      SCREEN_SHARE_PROFILES[id].maxHeight,
      SCREEN_SHARE_PROFILES[id].maxFramerate,
    );
  const current = settings.screenShareQuality;
  return {
    summary: t("voiceBar.screenSummary", {
      quality: t(SCREEN_LABEL[current]),
      detail: detailOf(current),
    }),
    groups: [
      choiceGroup(
        t("voiceBar.screenProfile"),
        SCREEN_ORDER,
        current,
        (id) => ({ label: t(SCREEN_LABEL[id]), detail: detailOf(id) }),
        (screenShareQuality) => change({ screenShareQuality }),
      ),
    ],
  };
}
