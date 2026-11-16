"use client";

export type HapticCue = "selection" | "light";

/*
 * Each cue is one short pulse: `selection` 8 ms at 30% strength, `light` 15 ms
 * at 40%. The Vibration API has no strength, so a weaker pulse runs the motor
 * for that share of its length and stays still for the rest: 2 ms on and 6 off,
 * 6 ms on and 9 off. The closing pause is silent; it keeps each pattern the
 * length of the pulse it stands for.
 */
const VIBRATION: Readonly<Record<HapticCue, number[]>> = {
  selection: [2, 6],
  light: [6, 9],
};

const SWITCH_ID = "haptic-switch";

/** The label that toggles the switch, shared by every cue once it exists. */
let switchLabel: HTMLLabelElement | null = null;

/*
 * Safari on iOS has no `navigator.vibrate`, but it plays the system haptic when
 * a switch control is toggled, and clicking the switch's label toggles it. The
 * label is `display: none` and has no text: an element that is not rendered is
 * outside the layout, the accessibility tree and the tab order, so nothing
 * shows it, announces it or focuses it.
 *
 * It is made on the first cue that needs it, not while rendering, and only
 * once `<body>` exists, so a cue that comes too early creates nothing and the
 * next one tries again. If anything takes the label out of the page, the next
 * cue puts the same one back rather than making another.
 */
function hapticSwitch(): HTMLLabelElement | null {
  const body = document.body;
  if (!body) return null;
  if (switchLabel === null) {
    const input = document.createElement("input");
    input.type = "checkbox";
    input.setAttribute("switch", "");
    input.id = SWITCH_ID;
    const label = document.createElement("label");
    label.htmlFor = SWITCH_ID;
    label.style.display = "none";
    label.append(input);
    switchLabel = label;
  }
  if (!switchLabel.isConnected) body.append(switchLabel);
  return switchLabel;
}

/**
 * Plays one cue from a deliberately small haptic vocabulary.
 *
 * Browsers with the Vibration API get a pattern; Safari on iOS gets the switch
 * above. Either way the browser may ignore a cue that does not follow a user
 * gesture. Haptics only enhance an interaction, so a browser or device failure
 * is swallowed rather than turning a successful click into an error, and
 * server rendering stays inert.
 */
export function triggerHaptic(cue: HapticCue): void {
  if (typeof window === "undefined") return;
  try {
    if (typeof navigator.vibrate === "function") {
      navigator.vibrate(VIBRATION[cue]);
      return;
    }
    hapticSwitch()?.click();
  } catch {
    // A locked-down or embedded browser can refuse either path.
  }
}
