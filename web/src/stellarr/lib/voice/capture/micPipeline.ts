import type { Translator } from "@/i18n";
import { translateForDocument } from "@/i18n/defaultLocale";
import { profileFor, usesWorkletSuppression } from "../audioProfiles";
import { buildCaptureGraph, type GraphDeps } from "./captureGraph";
import { MicCaptureError, type MicCapture } from "./micCapture";
import {
  browserGetUserMedia,
  openMicDevice,
  switchOnBrowserAudio,
  type BrowserFeature,
  type GetUserMedia,
} from "./micDevice";
import { wantsCaptureGraph, type VoiceSettings } from "../settings";

/**
 * Capture and processing for the outgoing microphone.
 *
 * This is the module the rest of the app imports. The device handling, the
 * browser fallbacks and the Web Audio graph live in sibling files and are
 * re-exported here, so a caller needs one path whatever it wants.
 */
export { MicCaptureError } from "./micCapture";
export type {
  ActiveAutoGain,
  ActiveNoiseSuppression,
  DetectorStages,
  MicCapture,
  MicFailureKind,
} from "./micCapture";
export {
  hasDeviceLabels,
  listInputDevices,
  listOutputDevices,
  listVideoDevices,
} from "./micDevices";

export interface CaptureDeps extends GraphDeps {
  getUserMedia?: GetUserMedia;
  /** Words the failures. Without one they follow the page's language. */
  translate?: Translator;
}

/**
 * Open the microphone and build the outgoing track for these settings.
 *
 * Throws `MicCaptureError` so the caller can tell "blocked" from "no device"
 * and say something useful in the person's language.
 */
export async function captureMicrophone(
  settings: VoiceSettings,
  deps: CaptureDeps = {},
): Promise<MicCapture> {
  const getUserMedia = deps.getUserMedia ?? browserGetUserMedia();
  const t = deps.translate ?? translateForDocument;
  if (!getUserMedia) {
    throw new MicCaptureError("unsupported", t("voiceClient.micUnsupported"));
  }

  const opened = await openMicDevice(settings, getUserMedia, t);
  const { stream: raw, track: rawTrack } = opened;
  const { contentHint } = profileFor(settings);
  rawTrack.contentHint = contentHint;

  const graph = wantsCaptureGraph(settings) ? await buildCaptureGraph(raw, settings, deps) : null;
  if (graph) graph.track.contentHint = contentHint;

  // The device was opened with the browser's suppressor and AGC off to make
  // room for ours. Whichever of ours is not running gets the browser's put
  // back, and what is reported is what the device confirms.
  const lost: BrowserFeature[] = [];
  if (usesWorkletSuppression(settings.noiseSuppression) && !graph?.suppressor) {
    lost.push("noiseSuppression");
  }
  if (settings.autoGainControl && !graph?.levelling) lost.push("autoGainControl");
  const { active: browser } = await switchOnBrowserAudio(opened, lost);

  let disposed = false;
  return {
    track: graph?.track ?? rawTrack,
    stream: graph?.stream ?? raw,
    monitorStream: graph?.monitorStream ?? raw,
    rawTrack,
    processed: graph !== null,
    noiseSuppression: graph?.suppressor ?? (browser.noiseSuppression ? "browser" : "none"),
    gateActive: graph?.gateActive ?? false,
    autoGain: graph?.levelling ? "worklet" : browser.autoGainControl ? "browser" : "none",
    setSensitivity: (sensitivity) => graph?.setSensitivity(sensitivity),
    setInputGain: (db) => graph?.setInputGain(db),
    fadeIn: () => graph?.fadeIn(),
    resume: async () => (graph ? graph.resume() : true),
    dispose() {
      if (disposed) return;
      disposed = true;
      graph?.dispose();
      for (const track of raw.getTracks()) track.stop();
    },
  };
}
