import { useEffect, useMemo, useState } from "react";

import type { VoiceSettings } from "@/ports";

/**
 * How long a noise suppression mode has to stay chosen before the audio
 * follows it. Arrow keys step through the modes in the settings and in the
 * microphone menu, so reaching Speex passes RNNoise and GTCRN on the way; each
 * mode passed would otherwise reopen the microphone and start a download of
 * its own, RNNoise's several megabytes among them.
 */
export const NOISE_SETTLE_MS = 700;

/**
 * The settings that running audio follows: these, except that a new noise
 * suppression mode reaches them only once it has stayed chosen for
 * `NOISE_SETTLE_MS`. The controls show every press at once; the capture and
 * the download wait for the one that stays. Everything else passes straight
 * through.
 */
export function useSettledVoiceSettings(settings: VoiceSettings): VoiceSettings {
  const chosen = settings.noiseSuppression;
  const [settled, setSettled] = useState(chosen);

  useEffect(() => {
    if (chosen === settled) return;
    const timer = setTimeout(() => setSettled(chosen), NOISE_SETTLE_MS);
    return () => clearTimeout(timer);
  }, [chosen, settled]);

  return useMemo(
    () => (chosen === settled ? settings : { ...settings, noiseSuppression: settled }),
    [settings, chosen, settled],
  );
}
