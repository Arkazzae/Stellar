// The fake microphones of the rail probe, as 48 kHz WAV files that Chrome
// loops for as long as the call lasts. Speech comes from WAV files made with
// the macOS voices (never committed, see bench/voice/make-tts.mjs); pauses are
// the fixtures' quiet room, and only the stretch meant to show what a call
// costs with nobody talking is digital silence.
//
//   node bench/rail/updates/mics.mjs <dir with a-speech.wav and b-speech.wav>
import { readFileSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";

import { runnerImport } from "vite";

const voiceRoot = resolve(import.meta.dirname, "..", "..", "voice");
const dir = resolve(process.argv[2] ?? ".");
const config = { configFile: false, logLevel: "error" };
const { module: fixtures } = await runnerImport(resolve(voiceRoot, "fixtures.ts"), config);
const { module: wav } = await runnerImport(resolve(voiceRoot, "wav.ts"), config);

const RATE = 48_000;
const speech = (name) => {
  const decoded = wav.decodeWav(new Uint8Array(readFileSync(join(dir, name))));
  if (decoded.sampleRate !== RATE) throw new Error(`${name} is not ${RATE} Hz`);
  return decoded.samples;
};
const room = (seconds) => fixtures.ROOM.room(Math.round(seconds * RATE), -70);
const zeros = (seconds) => new Float32Array(Math.round(seconds * RATE));

function write(name, parts) {
  const total = parts.reduce((sum, part) => sum + part.length, 0);
  const out = new Float32Array(total);
  let at = 0;
  for (const part of parts) {
    out.set(part, at);
    at += part.length;
  }
  writeFileSync(join(dir, name), wav.encodeWav(out, RATE));
  console.log(`${name}: ${(total / RATE).toFixed(1)} s`);
}

const a = speech("a-speech.wav");
const b = speech("b-speech.wav");
// A talks from the moment the microphone opens, pauses in a quiet room, then
// sits in digital silence: the stretch where only the call's own timers run.
write("mic-a.wav", [room(0.3), a, room(12), zeros(20)]);
// B talks right after joining, then pauses.
write("mic-b.wav", [room(0.5), b, room(20)]);
// C is a quiet room throughout.
write("mic-c.wav", [room(30)]);
process.exit(0);
