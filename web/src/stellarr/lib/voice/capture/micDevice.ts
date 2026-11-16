import type { Translator } from "@/i18n";
import { translateForDocument } from "@/i18n/defaultLocale";
import { MicCaptureError } from "./micCapture";
import type { VoiceSettings } from "../settings";

/**
 * The microphone device and what the browser does on it.
 *
 * This is the only place that asks the browser for its own processing (echo
 * cancellation, its noise suppressor, its gain control) and the only place
 * that reads back what the browser agreed to. Opening the device and switching
 * one of those features on later both go through `constraintsFor` and
 * `readBack`, so the request and the report cannot drift apart.
 */

export type GetUserMedia = (constraints: MediaStreamConstraints) => Promise<MediaStream>;

/**
 * Echo cancellation as the track reports it: off, the browser's ordinary kind,
 * or one of the named modes of the specification (`all` removes everything the
 * system plays, `remote-only` only what arrives over peer connections). The
 * named modes are reported, never asked for; see `requestFor`.
 */
export type ActiveEchoCancellation = "none" | "browser" | "all" | "remote-only";

/** What the browser itself is doing on the device. */
export interface BrowserAudio {
  echoCancellation: ActiveEchoCancellation;
  noiseSuppression: boolean;
  autoGainControl: boolean;
}

/** What the browser is asked for. Every constraint the device has comes from one of these. */
export interface BrowserAudioRequest {
  echoCancellation: boolean;
  noiseSuppression: boolean;
  autoGainControl: boolean;
  /** `null` takes whatever the system calls default. */
  deviceId: string | null;
}

/** The browser features a capture graph can hand back when its own stage fails. */
export type BrowserFeature = "noiseSuppression" | "autoGainControl";

/** An open device, what it was asked for and what it reports doing. */
export interface MicDevice {
  stream: MediaStream;
  track: MediaStreamTrack;
  requested: BrowserAudioRequest;
  active: BrowserAudio;
}

/**
 * What these settings ask of the browser.
 *
 * - Echo cancellation stays under the person's control. It is not a noise
 *   feature, and switching it off with speakers is how a call turns into a
 *   feedback loop. It is asked for as the plain boolean. Chromium also accepts
 *   the named modes `all` and `remote-only` (from Chrome 141), and neither is
 *   asked for. `remote-only` is the least `true` already promises. `all`
 *   cancels against a loopback of everything the system plays, which on macOS
 *   needs the System Audio Recording permission. Where Chrome may not open
 *   that loopback it still resolves `getUserMedia` and reports `all`, then
 *   ends the track a tenth of a second later, and a call built on it sends
 *   silence. No check made while opening can show that such a capture stays
 *   alive.
 * - Only the chosen suppressor is ever on. With a worklet suppressor the
 *   browser's is off rather than stacked underneath: two noise gates in series
 *   chew holes in speech, and the browser's would run first, so the worklet
 *   would only ever see already-damaged audio.
 * - The browser's gain control is off whatever the person chose. Automatic
 *   volume is our own leveller in the capture graph (see `micPipeline`): it
 *   learns only from vowels, so it cannot turn the room up between sentences
 *   the way the platform AGC does. The browser's is switched back on only when
 *   the graph cannot level.
 */
export function requestFor(settings: VoiceSettings): BrowserAudioRequest {
  return {
    echoCancellation: settings.echoCancellation,
    noiseSuppression: settings.noiseSuppression === "browser",
    autoGainControl: false,
    deviceId: settings.inputDeviceId,
  };
}

/**
 * The whole constraint set for a request: mono at 48 kHz to match Opus, and
 * an explicitly chosen device pinned exactly. `applyConstraints` replaces a
 * track's constraints wholesale, so a change is always sent as a full set.
 */
export function constraintsFor(request: BrowserAudioRequest): MediaTrackConstraints {
  return {
    echoCancellation: request.echoCancellation,
    autoGainControl: request.autoGainControl,
    noiseSuppression: request.noiseSuppression,
    channelCount: 1,
    sampleRate: 48_000,
    ...(request.deviceId ? { deviceId: { exact: request.deviceId } } : {}),
  };
}

/** The spec's string modes are not in the DOM typings, which only know booleans. */
type ReportedSettings = {
  echoCancellation?: unknown;
  noiseSuppression?: unknown;
  autoGainControl?: unknown;
};

function reportedSettings(track: MediaStreamTrack): ReportedSettings {
  try {
    return (track.getSettings?.() as ReportedSettings | undefined) ?? {};
  } catch {
    // Some browser/device pairs throw once a track changes state.
    return {};
  }
}

function reportedEcho(reported: unknown, asked: boolean): ActiveEchoCancellation {
  if (reported === "all" || reported === "remote-only") return reported;
  if (typeof reported === "boolean") return reported ? "browser" : "none";
  return asked ? "browser" : "none";
}

function reportedFlag(reported: unknown, asked: boolean): boolean {
  return typeof reported === "boolean" ? reported : asked;
}

/**
 * What the browser says it is doing, feature by feature. A value it reports
 * is taken as it is, whatever was asked; a value it does not report is taken
 * as granted, since the request that set it was accepted.
 */
export function readBack(track: MediaStreamTrack, request: BrowserAudioRequest): BrowserAudio {
  const reported = reportedSettings(track);
  return {
    echoCancellation: reportedEcho(reported.echoCancellation, request.echoCancellation),
    noiseSuppression: reportedFlag(reported.noiseSuppression, request.noiseSuppression),
    autoGainControl: reportedFlag(reported.autoGainControl, request.autoGainControl),
  };
}

/** The browser's own `getUserMedia`, or null where there is none (insecure origin, old browser). */
export function browserGetUserMedia(): GetUserMedia | null {
  if (typeof navigator === "undefined" || !navigator.mediaDevices?.getUserMedia) return null;
  return (constraints) => navigator.mediaDevices.getUserMedia(constraints);
}

function failureFor(cause: unknown, t: Translator): MicCaptureError {
  const name = typeof cause === "object" && cause !== null && "name" in cause ? cause.name : "";
  if (name === "NotAllowedError" || name === "SecurityError") {
    return new MicCaptureError("denied", t("voiceClient.micDenied"));
  }
  if (name === "OverconstrainedError" || name === "NotFoundError") {
    return new MicCaptureError("unavailable", t("voiceClient.micGone"));
  }
  return new MicCaptureError("unavailable", t("voiceClient.micMissing"));
}

/**
 * Open the device for these settings, once, and say what the browser is doing
 * on it.
 *
 * Throws `MicCaptureError`, so the caller can tell "blocked" from "no device"
 * and say something useful, worded by `t` (the page's language without one).
 * A stream that arrives without an audio track is stopped before the error is
 * thrown.
 */
export async function openMicDevice(
  settings: VoiceSettings,
  getUserMedia: GetUserMedia,
  t: Translator = translateForDocument,
): Promise<MicDevice> {
  const requested = requestFor(settings);
  let stream: MediaStream;
  try {
    stream = await getUserMedia({ audio: constraintsFor(requested), video: false });
  } catch (cause) {
    throw failureFor(cause, t);
  }

  const track = stream.getAudioTracks()[0];
  if (!track) {
    for (const other of stream.getTracks()) other.stop();
    throw new MicCaptureError("unavailable", t("voiceClient.micMissing"));
  }
  return { stream, track, requested, active: readBack(track, requested) };
}

/**
 * Switch some of the browser's own processing on, in place, on a device that
 * is already carrying the call, after the graph stage meant to do the same job
 * failed.
 *
 * Best effort. The device keeps working whatever happens: a browser that
 * refuses the change leaves the track as it was, so the answer is then the
 * device as it was; one that accepts it may still do nothing, which the read
 * back shows. Unprocessed audio is worse than processed audio, but no audio is
 * worse than both.
 */
export async function switchOnBrowserAudio(
  device: MicDevice,
  features: readonly BrowserFeature[],
): Promise<MicDevice> {
  const wanted = features.filter((feature) => !device.requested[feature]);
  if (wanted.length === 0) return device;
  if (typeof device.track.applyConstraints !== "function") return device;

  const next: BrowserAudioRequest = { ...device.requested };
  for (const feature of wanted) next[feature] = true;
  try {
    await device.track.applyConstraints(constraintsFor(next));
  } catch {
    return device;
  }
  return { ...device, requested: next, active: readBack(device.track, next) };
}
