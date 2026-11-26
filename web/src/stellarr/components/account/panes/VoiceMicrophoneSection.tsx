import { useEffect, useId, useRef } from "react";

import { Button } from "@stellarr/ui";
import { Icon } from "@stellarr/icons";
import { ErrorNotice } from "@/components/primitives/ErrorNotice";
import type { KeyChord, KeybindRecordingError } from "@/lib/keybinds/keybinds";
import {
  INPUT_GAIN_MAX_DB,
  INPUT_GAIN_MIN_DB,
  INPUT_GAIN_STEP_DB,
  INPUT_MODE_HINT,
  INPUT_MODE_LABEL,
} from "@/ports";
import type { InputMode, MicTest, VoiceSettings } from "@/ports";
import { useLanguage } from "@/state/hooks";

import { BindingControls } from "./KeybindsPane";
import { DeviceSelect, FormRow, RangeField, SwitchRow, formatDb } from "./VoiceFields";
import { Ladder, type Rung } from "./VoiceLadder";

/** The talk key, recorded here as it is in the shortcuts. */
export interface TalkKeyControl {
  binding: KeyChord | null;
  /** The next key press becomes the talk key. */
  listening: boolean;
  /** Why the last key pressed for it was refused. */
  error: KeybindRecordingError | null;
  onRecord: () => void;
  onClear: () => void;
}

export interface VoiceMicrophoneSectionProps {
  settings: VoiceSettings;
  onChange: (patch: Partial<VoiceSettings>) => void;
  /** Persists the already-applied sensitivity in the signed-in account. */
  onVadCommit: (value: number) => void;
  mic: MicTest;
  /** Without it push to talk can be chosen here, but its key only under the shortcuts. */
  talkKey?: TalkKeyControl;
}

const INPUT_MODE_ORDER: readonly InputMode[] = ["voiceActivity", "pushToTalk"];

const INPUT_MODE_RUNGS: Record<InputMode, Rung> = {
  voiceActivity: { label: INPUT_MODE_LABEL.voiceActivity, hint: INPUT_MODE_HINT.voiceActivity },
  pushToTalk: { label: INPUT_MODE_LABEL.pushToTalk, hint: INPUT_MODE_HINT.pushToTalk },
};

/**
 * Which microphone, when it sends and how loud speech has to be: the first
 * rows of the "your voice" group. Whether it works is `MicrophoneCheck`, in
 * the pane's check panel, which stays in view while these rows are adjusted:
 * "is it loud enough" and "is it speech" are two questions its one bar
 * answers.
 *
 * Push to talk replaces the gate's row with the talk key's: with the key
 * deciding when the microphone sends, the gate has nothing left to decide.
 */
export function VoiceMicrophoneSection({
  settings,
  onChange,
  onVadCommit,
  mic,
  talkKey,
}: VoiceMicrophoneSectionProps) {
  const { t } = useLanguage();
  const deviceId = useId();
  const vadId = useId();
  const gainId = useId();
  const pushToTalk = settings.inputMode === "pushToTalk";

  // The sensitivity is applied on every step but saved to the account when the
  // gesture ends. By then the parent may not have re-rendered with the last
  // step, so the value being dragged is kept here rather than read from props.
  const draggedVad = useRef(settings.vadSensitivity);
  useEffect(() => {
    draggedVad.current = settings.vadSensitivity;
  }, [settings.vadSensitivity]);

  return (
    <>
      <FormRow
        label={t("voice.device")}
        detail={t("voice.deviceImmediateDetail")}
        htmlFor={deviceId}
      >
        <DeviceSelect
          id={deviceId}
          label={t("voice.microphone")}
          devices={mic.devices}
          value={settings.inputDeviceId}
          defaultLabel={t("voice.systemDefault")}
          onChange={(inputDeviceId) => onChange({ inputDeviceId })}
        />
        {/* Before permission the browser hands out entries with empty
            labels, so the list is numbered and says why. */}
        {mic.labelsKnown ? null : (
          <p className="pf-count">{t("voice.microphoneNamesHint")}</p>
        )}
        {/* The system default names a rule, not a microphone. While the test
            is open the rule has picked one, and that is the part people are
            surprised by. */}
        {settings.inputDeviceId === null && mic.activeDeviceLabel !== null ? (
          <p className="pf-count">
            {t("voice.currentDevice", { device: mic.activeDeviceLabel })}
          </p>
        ) : null}
      </FormRow>

      <div className="frm-row">
        <span className="frm-lab">{t("voice.inputMode")}</span>
        <div className="frm-ctl">
          <Ladder
            order={INPUT_MODE_ORDER}
            rungs={INPUT_MODE_RUNGS}
            value={settings.inputMode}
            onChange={(inputMode) => onChange({ inputMode })}
            label={t("voice.inputMode")}
          />
        </div>
      </div>

      {pushToTalk && talkKey !== undefined ? (
        <FormRow label={t("voice.talkKey")} detail={t("voice.talkKeyDetail")}>
          <BindingControls
            label={t("voice.talkKey")}
            binding={talkKey.binding}
            listening={talkKey.listening}
            error={talkKey.error}
            onRecord={talkKey.onRecord}
            onClear={talkKey.onClear}
          />
        </FormRow>
      ) : null}
      {/* Push to talk without a key sends nothing at all, which from the
          other side is a call where you never speak. */}
      {pushToTalk && talkKey?.binding === null && !talkKey.listening ? (
        <p className="acc-answer bad mic-truth" role="status">
          {t("voice.talkKeyMissing")}
        </p>
      ) : null}

      <FormRow
        label={t("voice.speechSensitivity")}
        detail={t("voice.speechSensitivityDetail")}
        htmlFor={vadId}
      >
        <RangeField
          id={vadId}
          min={0}
          max={100}
          step={5}
          value={settings.vadSensitivity}
          display={String(settings.vadSensitivity)}
          onChange={(vadSensitivity) => {
            draggedVad.current = vadSensitivity;
            onChange({ vadSensitivity });
          }}
          onRelease={() => onVadCommit(draggedVad.current)}
        />
      </FormRow>

      <FormRow
        label={t("voice.inputGain")}
        detail={t("voice.inputGainDetail")}
        htmlFor={gainId}
      >
        <RangeField
          id={gainId}
          min={INPUT_GAIN_MIN_DB}
          max={INPUT_GAIN_MAX_DB}
          step={INPUT_GAIN_STEP_DB}
          value={settings.inputGainDb}
          display={formatDb(settings.inputGainDb)}
          onChange={(inputGainDb) => onChange({ inputGainDb })}
        />
      </FormRow>

      {pushToTalk ? null : (
        <SwitchRow
          label={t("voice.inputGate")}
          detail={t("voice.inputGateDetail")}
          checked={settings.inputGate}
          onChange={(inputGate) => onChange({ inputGate })}
        />
      )}
      {/* A switch that is on while the browser cannot build the gate would
          be a lie, the same as the suppressor line further down. */}
      {!pushToTalk && settings.inputGate && mic.gateActive === false ? (
        <p className="acc-answer bad mic-truth" role="status">
          {t("voice.inputGateUnavailable")}
        </p>
      ) : null}
    </>
  );
}

/**
 * Whether the microphone works: the test, its level meter and its verdict.
 * In push to talk speech goes out only while the key is held, so the verdict
 * says that rather than that it would be sent.
 */
export function MicrophoneCheck({ mic, pushToTalk = false }: { mic: MicTest; pushToTalk?: boolean }) {
  const { t } = useLanguage();
  const level = Math.round(mic.level * 100);

  let status = "";
  if (mic.error === null) {
    if (!mic.running) status = t("voice.testOff");
    else if (!mic.speaking) status = t("voice.silence");
    else status = pushToTalk ? t("voice.hearYouPushToTalk") : t("voice.hearYou");
  }

  return (
    <>
      <div className="mic-test">
        <Button
          type="button"
          variant="secondary"
          size="sm"
          className={mic.running ? "test-toggle on" : "test-toggle"}
          disabled={mic.starting}
          onClick={() => (mic.running ? mic.stop() : mic.start())}
        >
          <Icon name={mic.running ? "pause" : "mic"} size={15} />
          {mic.running
            ? t("voice.stop")
            : mic.starting
              ? t("voice.opening")
              : t("voice.checkMicrophone")}
        </Button>
        {/* One bar rather than a row of blocks: the number that matters is how
            close the level is to the line the detector draws, and `speaking`
            is where that line landed. */}
        <div
          className={mic.speaking ? "mic-bar live" : "mic-bar"}
          role="meter"
          aria-label={t("voice.microphoneLevel")}
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={level}
        >
          <i style={{ width: `${level}%` }} />
          {/* The detector publishes in eighths, so without a mark that holds
              the loudest moment, one loud syllable is a frame you miss. */}
          {mic.peak > 0 ? <u style={{ left: `${Math.round(mic.peak * 100)}%` }} /> : null}
        </div>
      </div>
      <p className="pf-count" role="status">
        {status}
      </p>
      {mic.error === null ? null : (
        <ErrorNotice
          compact
          title={t("voice.microphoneNotWorking")}
          message={mic.error}
          icon="micSlash"
          actions={[{ label: t("common.tryAgain"), onClick: mic.start }]}
        />
      )}
    </>
  );
}
