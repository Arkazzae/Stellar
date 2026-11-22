import { useState, type KeyboardEvent } from "react";

import { Icon, type IconName } from "@stellarr/icons";
import type { CallVM } from "@/components/view-models";
import { SoundboardPopover } from "@/components/voice/SoundboardPopover";
import type { MessageKey } from "@/i18n";
import type { Sound, SoundboardLive, SuppressorDownload, VoiceSettings } from "@/ports";
import { useLanguage } from "@/state/hooks";

import { CallClock } from "./CallClock";
import { MicMenu } from "./MicMenu";
import { IDLE_SOUNDBOARD } from "./soundboardLive";

/** How a status word is coloured: waiting, a warning, a failure, or your own share. */
type Tone = "busy" | "warn" | "bad" | "acc";

/**
 * What the dock and the call view say in place of the call's time, for every
 * state other than plain conversation. Words only when something is not as it
 * should be, or when your screen is out.
 */
export const CALL_STATUS: Record<
  Exclude<CallVM["headline"], "connected">,
  { words: MessageKey; tone: Tone }
> = {
  "requesting-mic": { words: "shell.voiceRequestingMic", tone: "busy" },
  connecting: { words: "shell.voiceConnecting", tone: "busy" },
  reconnecting: { words: "shell.voiceReconnecting", tone: "warn" },
  degraded: { words: "shell.voiceDegraded", tone: "warn" },
  failed: { words: "shell.voiceFailed", tone: "bad" },
  screen: { words: "shell.voiceScreen", tone: "acc" },
};

const RINGING = { words: "dm.calling", tone: "busy" } as const satisfies {
  words: MessageKey;
  tone: Tone;
};

const QUALITY: Record<Exclude<CallVM["connectionQuality"], "unknown">, MessageKey> = {
  good: "shell.voiceQualityGood",
  fair: "shell.voiceQualityFair",
  poor: "shell.voiceQualityPoor",
};

/** The logo's three bars: how many are lit, and in which colour, by state. */
type Bars = "q3" | "q2" | "q1" | "busy" | "off" | "screen";

const QUALITY_BARS: Record<CallVM["connectionQuality"], Bars> = {
  good: "q3",
  fair: "q2",
  poor: "q1",
  unknown: "busy",
};

function barsFor(headline: CallVM["headline"], quality: CallVM["connectionQuality"]): Bars {
  switch (headline) {
    case "screen":
      return "screen";
    case "connecting":
    case "requesting-mic":
      return "busy";
    case "reconnecting":
    case "failed":
      return "off";
    // Unsteady media is a warning whatever the last reading said: green bars
    // beside the warning words would contradict them.
    case "degraded":
      return "q2";
    default:
      return QUALITY_BARS[quality];
  }
}

export interface VoiceDockProps {
  /** No call means the dock is present but `.hidden`, as in the mockup. */
  call: CallVM | null;
  /**
   * A private call the other person has not joined yet. Once connected, the
   * dock says it is calling them, and keeps no time: nothing has started.
   */
  ringing?: boolean;
  /**
   * The call's own view is on screen, with the bar under it: the dock keeps
   * its status line and leaves the controls to the bar.
   */
  collapsed?: boolean;
  sounds: Sound[];
  /** False where this person may not play the board: its pads are locked and it says why. */
  soundboardSupported?: boolean;
  /** What plays in the room and the cooldown, read by the soundboard panel alone. */
  soundboardLive?: SoundboardLive;
  /** For people who manage the server: its settings, on the soundboard. */
  onManageSounds?: () => void;
  /** Device settings: the microphone menu edits them, the soundboard reads its volume. */
  settings: VoiceSettings;
  onChangeSettings: (patch: Partial<VoiceSettings>) => void;
  /** The chosen suppressor's files on their way: a ring on the microphone's arrow. */
  suppressorDownload?: SuppressorDownload | null;
  /** The voice settings dialog, from the microphone menu's last row. */
  onOpenSoundSettings: () => void;
  onToggleMute: () => void;
  onToggleDeafen: () => void;
  onToggleCamera: () => void;
  cameraSupported?: boolean;
  onToggleShare: () => void;
  onLeave: () => void;
  /** Absent where the call has no soundboard, which is a private one: the dock then has no trigger. */
  onPlaySound?: (soundId: string) => void;
  /** Clicking the status while your screen is out jumps back to the channel. */
  onReturnToChannel: () => void;
}

interface DockToggleProps {
  icon: IconName;
  pressed: boolean;
  /** `muted` for controls whose pressed state silences something, `on` for the rest. */
  pressedClass: "muted" | "on";
  title: string;
  label: string;
  disabled?: boolean;
  busy?: boolean;
  onClick: () => void;
}

function DockToggle({
  icon,
  pressed,
  pressedClass,
  title,
  label,
  disabled,
  busy,
  onClick,
}: DockToggleProps) {
  return (
    <button
      type="button"
      className={pressed ? `dock-btn ${pressedClass}` : "dock-btn"}
      aria-pressed={pressed}
      title={title}
      aria-label={label}
      aria-busy={busy || undefined}
      disabled={disabled}
      onClick={onClick}
    >
      <Icon name={icon} size={16} />
    </button>
  );
}

type LiveDockProps = Omit<VoiceDockProps, "call"> & { call: CallVM };

/**
 * The call's controls in the bar's groups: what you say and hear, then what
 * you show. Its own component, so the panels it opens close with it when the
 * dock folds down to its status line.
 */
function DockControls({
  call,
  sounds,
  soundboardSupported = true,
  soundboardLive = IDLE_SOUNDBOARD,
  onManageSounds,
  settings,
  onChangeSettings,
  suppressorDownload = null,
  onOpenSoundSettings,
  onToggleMute,
  onToggleDeafen,
  onToggleCamera,
  cameraSupported = false,
  onToggleShare,
  onPlaySound,
}: LiveDockProps) {
  const { t } = useLanguage();
  const [panel, setPanel] = useState<"mic" | "board" | null>(null);
  const panelState = (id: "mic" | "board") => ({
    open: panel === id,
    onOpenChange: (open: boolean) => setPanel(open ? id : null),
  });

  const cameraLabel = call.cameraPending
    ? t("voiceBar.cameraStarting")
    : call.cameraOn
      ? t("voiceBar.cameraStop")
      : t("voiceBar.cameraStart");

  return (
    <div className="dock-ctrl">
      <span className="dock-grp">
        <DockToggle
          icon={call.muted ? "micSlash" : "mic"}
          pressed={call.muted}
          pressedClass="muted"
          title={call.muted ? t("voiceBar.unmute") : t("voiceBar.mute")}
          label={t("voice.microphone")}
          onClick={onToggleMute}
        />
        <MicMenu
          {...panelState("mic")}
          settings={settings}
          download={suppressorDownload}
          onChangeSettings={onChangeSettings}
          onMoreSettings={onOpenSoundSettings}
          caretClassName="dock-btn dock-caret"
        />
        <DockToggle
          icon={call.deafened ? "headphonesSlash" : "headphones"}
          pressed={call.deafened}
          pressedClass="muted"
          title={call.deafened ? t("shell.unmuteSound") : t("shell.muteSound")}
          label={t("shell.muteSound")}
          onClick={onToggleDeafen}
        />
      </span>
      <span className="dock-grp">
        <DockToggle
          icon="camera"
          pressed={call.cameraOn}
          pressedClass="on"
          title={cameraSupported ? cameraLabel : t("voiceBar.cameraUnsupported")}
          label={cameraLabel}
          busy={call.cameraPending}
          disabled={!cameraSupported || call.cameraPending}
          onClick={onToggleCamera}
        />
        <DockToggle
          icon="screencast"
          pressed={call.sharing}
          pressedClass="on"
          title={call.sharing ? t("shell.stopSharing") : t("voiceBar.shareScreen")}
          label={t("voiceBar.shareScreen")}
          onClick={onToggleShare}
        />
        {onPlaySound === undefined ? null : (
          <SoundboardPopover
            sounds={sounds}
            live={soundboardLive}
            canPlay={soundboardSupported}
            {...panelState("board")}
            onPlaySound={onPlaySound}
            onManage={onManageSounds}
            volume={settings.soundboardVolume}
            onVolumeChange={(soundboardVolume) => onChangeSettings({ soundboardVolume })}
            renderTrigger={(trigger) => (
              <button
                type="button"
                className="dock-btn"
                title={t("voice.soundboard")}
                aria-label={t("voice.soundboard")}
                {...trigger}
              >
                <Icon name="board" size={16} />
              </button>
            )}
          />
        )}
      </span>
    </div>
  );
}

function LiveDock(props: LiveDockProps) {
  const { call, ringing = false, collapsed = false, onLeave, onReturnToChannel } = props;
  const { t } = useLanguage();

  const status =
    call.headline === "connected" ? (ringing ? RINGING : null) : CALL_STATUS[call.headline];
  const bars = barsFor(call.headline, call.connectionQuality);
  // While the bars stand for the link, its quality and ping are said in words
  // for a pointer and a screen reader.
  const describesQuality = call.headline === "connected" || call.headline === "degraded";
  const qualityDetail =
    call.connectionQuality === "unknown" || call.latencyMs === null
      ? t("shell.voiceQualityMeasuringDetail")
      : t("shell.voiceQualityDetail", {
          quality: t(QUALITY[call.connectionQuality]),
          latency: Math.round(call.latencyMs),
        });

  // Your screen is out, so the status is a way back to it, unless the call's
  // view is already where you are.
  const live = call.headline === "screen" && !collapsed;

  const line = (
    <>
      <span
        className={`qbars ${bars}`}
        {...(describesQuality
          ? { role: "img", title: qualityDetail, "aria-label": qualityDetail }
          : { "aria-hidden": true })}
      >
        <i />
        <i />
        <i />
      </span>
      <span className="ds-name">{call.channelName}</span>
      {status === null ? (
        <span className="ds-time">
          <CallClock since={call.startedAt} />
        </span>
      ) : (
        <span className={`ds-time ${status.tone}`}>{t(status.words)}</span>
      )}
    </>
  );

  const returnOnKey = (event: KeyboardEvent<HTMLElement>) => {
    if (event.key !== "Enter" && event.key !== " ") return;
    event.preventDefault();
    onReturnToChannel();
  };

  return (
    <div className={collapsed ? "voice-dock collapsed" : "voice-dock"} role="status">
      <div className="dock-status">
        {live ? (
          <div
            className="ds-link"
            role="button"
            tabIndex={0}
            title={t("shell.returnToChannel")}
            onClick={onReturnToChannel}
            onKeyDown={returnOnKey}
          >
            {line}
            <Icon name="arrowRight" size={14} className="ds-arrow" />
          </div>
        ) : (
          line
        )}
        <button
          type="button"
          className="dock-hang"
          title={t("voiceBar.disconnect")}
          aria-label={t("voiceBar.disconnect")}
          onClick={onLeave}
        >
          <Icon name="phoneX" size={16} />
        </button>
      </div>
      {collapsed ? null : <DockControls {...props} />}
    </div>
  );
}

/** The call in progress, above the account row: where you are, how it goes, and the controls. */
export function VoiceDock({ call, ...rest }: VoiceDockProps) {
  if (call === null) return <div className="voice-dock hidden" role="status" />;
  return <LiveDock call={call} {...rest} />;
}
