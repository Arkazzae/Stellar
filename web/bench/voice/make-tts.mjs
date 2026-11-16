// Makes a set of synthetic speech recordings with the macOS system voices,
// for when no recording of a real voice is at hand. They go into the ignored
// recordings/ folder and must stay there: the licence of the system voices
// does not allow their output to be redistributed, so neither these files nor
// anything made from their samples may be committed.
//
//   node bench/voice/make-tts.mjs            make the files that are missing
//   node bench/voice/make-tts.mjs --force    make all of them again
//
// Needs `say` and `afconvert`, which come with macOS.
import { spawnSync } from "node:child_process";
import { existsSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

const recordings = resolve(import.meta.dirname, "recordings");
const force = process.argv.includes("--force");

/** `[[slnc n]]` is a pause of n ms in `say`'s own markup. */
const pause = (ms) => ` [[slnc ${ms}]] `;

const SET = [
  {
    file: "pl-zosia-speech.wav",
    voice: "Zosia",
    idea: "Polish sentences with pauses of 0.7 to 2 s, many starting with sz, cz, prz or a plosive",
    text: [
      "Dzień dobry, czy wszyscy mnie dobrze słyszą?",
      pause(1200),
      "Szybko sprawdzę, czy mikrofon działa jak należy.",
      pause(700),
      "Przepraszam, chwileczkę, ktoś dzwoni do drzwi.",
      pause(2000),
      "Już jestem. Wracając do tematu, plan na ten tydzień jest prosty.",
      pause(900),
      "Trzeba skończyć testy, poprawić błędy i przygotować wydanie.",
      pause(1500),
      "Czy ktoś ma jakieś pytania?",
      pause(1000),
      "Świetnie, w takim razie kończymy na dzisiaj.",
    ].join(" "),
  },
  {
    file: "pl-zosia-fast.wav",
    voice: "Zosia",
    rate: 260,
    idea: "fast Polish with no pauses",
    text: "Szczerze mówiąc, nie wiem, czy zdążymy, bo przecież wszystko trzeba jeszcze raz sprawdzić, przetestować, poprawić i wysłać przed piątkiem, a czasu jest naprawdę niewiele.",
  },
  {
    file: "en-samantha-speech.wav",
    voice: "Samantha",
    idea: "a female English voice, sentences with pauses of 0.7 to 2 s",
    text: [
      "Hi everyone, can you hear me all right?",
      pause(1100),
      "Let me share my screen for a second.",
      pause(800),
      "So, the first thing on the list is the release.",
      pause(1900),
      "Sorry, the dog was barking. Where was I?",
      pause(900),
      "Right. We still have two bugs to fix before Friday.",
      pause(1400),
      "Fine, thanks everybody, talk to you tomorrow.",
    ].join(" "),
  },
  {
    file: "en-daniel-speech.wav",
    voice: "Daniel",
    idea: "a male English voice: very short words alone, then one long sentence",
    text: [
      "Yes.",
      pause(900),
      "No.",
      pause(1100),
      "Stop.",
      pause(800),
      "Go.",
      pause(1300),
      "Okay.",
      pause(1500),
      "What I was trying to say before the connection dropped is that the whole thing works much better when everybody keeps their microphone muted while somebody else is presenting.",
    ].join(" "),
  },
  {
    file: "en-whisper.wav",
    voice: "Whisper",
    idea: "the whisper voice, unpitched, with pauses",
    text: [
      "Can you hear me? I have to keep my voice down.",
      pause(1500),
      "The baby is sleeping in the next room.",
      pause(2000),
      "I will speak up again in a minute.",
    ].join(" "),
  },
];

function tool(name) {
  return spawnSync("/usr/bin/which", [name]).status === 0;
}

if (!tool("say") || !tool("afconvert")) {
  console.error("This needs macOS's say and afconvert.");
  process.exit(1);
}

const work = mkdtempSync(join(tmpdir(), "stellarr-voice-tts-"));
try {
  for (const item of SET) {
    const target = join(recordings, item.file);
    if (existsSync(target) && !force) {
      console.log(`${item.file}: already there`);
      continue;
    }
    const aiff = join(work, item.file.replace(/\.wav$/, ".aiff"));
    const say = ["-v", item.voice, ...(item.rate ? ["-r", String(item.rate)] : []), "-o", aiff, item.text];
    if (spawnSync("say", say, { stdio: "inherit" }).status !== 0) throw new Error(`say failed for ${item.file}`);
    const convert = ["-f", "WAVE", "-d", "LEI16@48000", "-c", "1", aiff, target];
    if (spawnSync("afconvert", convert, { stdio: "inherit" }).status !== 0) {
      throw new Error(`afconvert failed for ${item.file}`);
    }
    // A record made from the old file would no longer describe this one.
    for (const suffix of [".capture.txt", ".playback.txt", ".summary.txt"]) {
      rmSync(target.replace(/\.wav$/, suffix), { force: true });
    }
    console.log(`${item.file}: ${item.idea}`);
  }
} finally {
  rmSync(work, { recursive: true, force: true });
}
