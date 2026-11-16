import { PROFILES, type TrigramProfile } from "./languageProfiles";
import { WORDS, type WordListLanguage } from "./languageWords";

/**
 * Which of the two chat languages a message is written in, or `und`.
 *
 * Chat translation knows Polish and English only, so nothing else is named: a
 * message in any other language, or with too little language in it to tell,
 * is `und`. Three kinds of evidence are weighed:
 *
 * - character trigrams, scored against profiles of folded Polish and English
 *   (languageProfiles.ts), which tell the two apart on any wording and say how
 *   much a message reads like either of them at all;
 * - short lists of function and chat words of Polish, English and the
 *   languages most easily taken for them (languageWords.ts), which also tell a
 *   Polish message from a Czech or Slovak one that looks much the same;
 * - letters: the ones only Polish writes count for it, the ones neither
 *   language writes (Czech ě and ř, German ä and ß, Spanish ñ) count against
 *   both, and a message mostly in another script is neither.
 *
 * Everything is folded first (lower case, no diacritics), so Polish typed
 * without its diacritics reads as Polish. The weights below were set by
 * measuring the detector on the interface catalogues and on chat messages
 * written for the purpose in Polish, English and two dozen other languages.
 *
 * Synchronous and cheap, a few thousand map lookups for the longest message,
 * so it runs on every message with nothing to load.
 */
export type DetectedLanguage = "pl" | "en" | "und";

/** Enough of a long message to decide on; the rest only costs time. */
const MAX_CHARACTERS = 2_000;

// Below two words or eight letters even a reader hesitates ("ok", "no to"),
// and an undetermined message is still translated, from the language it most
// likely is in. Above that the evidence rules below abstain on their own.
const MIN_LETTERS = 8;
const MIN_WORDS = 2;

/* ------------------------------------------------------------ stripping */

// Whatever a stranger typed is read here on the main thread, so every pattern
// below runs in time linear in the message: the text is cut at whitespace
// first, and each piece is judged on its own with patterns that cannot
// backtrack across it.

/** A code span, which may hold spaces. A fence that never closes is left alone. */
const CODE_SPAN = /(`+)[^`]*\1/g;

/**
 * A piece between spaces that is no word of the language: an address or path
 * (a slash), a mention or e-mail address (an at sign), a custom emoji, a file
 * or domain name (a dot between letters), anything with a digit.
 */
const NOT_A_WORD = /[@/\p{N}]|\p{L}\.\p{L}|^www\.|:[a-z0-9_-]{2,64}:/iu;

/** Formatting markers and escapes, around or inside a word. */
const MARKUP = /[\\*_~|[\]]/g;

function stripNonLanguage(text: string): string {
  return (
    text
      .replace(/[’‘ʼ]/g, "'")
      .replace(CODE_SPAN, " ")
      // A link's label is language and its target is not; apart, the target
      // goes with the rest of the addresses.
      .replaceAll("](", "] (")
      .split(/\s+/)
      .filter((piece) => piece !== "" && !NOT_A_WORD.test(piece))
      .join(" ")
      .replace(MARKUP, " ")
      .toLowerCase()
  );
}

/** Without diacritics. The Polish l with a stroke has none to drop, so it is mapped. */
function fold(text: string): string {
  return text.replace(/ł/g, "l").normalize("NFD").replace(/\p{M}/gu, "");
}

/* ------------------------------------------------------------ letters */

/** Letters of Polish that English lacks; o with an acute is too widely shared. */
const POLISH_LETTERS = /[ąćęłńśźż]/g;

/** Letters neither language writes natively. English borrows a few (café). */
const FOREIGN_LETTERS = /[àáâãäåāăèéêëěėēìíîïīıòôõöőøùúûüůűūýÿçčďđğľĺňřšşșťțžßæœñ¿¡]/g;

const count = (text: string, pattern: RegExp) => text.match(pattern)?.length ?? 0;

/* ------------------------------------------------------------ words */

type Language = WordListLanguage;

const LANGUAGES = Object.keys(WORDS) as Language[];

/** Each listed word to the languages that list it. */
const LISTED = new Map<string, Language[]>();
for (const language of LANGUAGES) {
  for (const word of new Set(WORDS[language].trim().split(/\s+/))) {
    LISTED.set(word, [...(LISTED.get(word) ?? []), language]);
  }
}

/** don't, I'll, it's, we're, I've, he'd, I'm */
const ENGLISH_CONTRACTION = /^[a-z]+'(?:t|ll|s|re|ve|d|m)$/;
/** l'appli, j'ai, qu'il, dell'app, c'era */
const ROMANCE_ELISION = /^(?:l|d|j|n|s|m|t|c|qu|lorsqu|puisqu|jusqu|dell|nell|all|dall|sull|un)'[a-z]/;

/* ------------------------------------------------------------ trigrams */

interface Scorer {
  logProbability: Map<string, number>;
  unseen: number;
}

function scorerOf({ total, levels }: TrigramProfile): Scorer {
  const logProbability = new Map<string, number>();
  let lowest = 0;
  for (const [level, grams] of levels) {
    // The middle of the level's count range, as a share of all trigrams.
    const value = Math.log((2 ** level * 1.5) / total);
    lowest = Math.min(lowest, value);
    for (let i = 0; i + 3 <= grams.length; i += 3) {
      logProbability.set(grams.slice(i, i + 3).replace(/_/g, " "), value);
    }
  }
  // A trigram outside the profile is rarer than any inside it.
  return { logProbability, unseen: lowest - Math.log(4) };
}

let scorers: Record<"pl" | "en", Scorer> | null = null;

/** Mean log-likelihood per trigram of the words under each profile. */
function trigramFit(words: readonly string[]): Record<"pl" | "en", number> {
  scorers ??= { pl: scorerOf(PROFILES.pl), en: scorerOf(PROFILES.en) };
  const sums = { pl: 0, en: 0 };
  let grams = 0;
  for (const word of words) {
    const padded = ` ${word.replace(/'/g, "")} `;
    for (let i = 0; i + 3 <= padded.length; i += 1) {
      const gram = padded.slice(i, i + 3);
      grams += 1;
      for (const language of ["pl", "en"] as const) {
        const { logProbability, unseen } = scorers[language];
        sums[language] += logProbability.get(gram) ?? unseen;
      }
    }
  }
  return { pl: sums.pl / grams, en: sums.en / grams };
}

/* ------------------------------------------------------------ evidence */

interface Evidence {
  polishLetters: number;
  foreignLetters: number;
  /** Listed words per language, a shared word counting for each a fraction. */
  hits: Record<Language, number>;
  contractions: number;
  elisions: number;
  fit: Record<"pl" | "en", number>;
}

function evidenceOf(text: string): Evidence | null {
  const stripped = stripNonLanguage(text.slice(0, MAX_CHARACTERS));
  const letters = stripped.match(/\p{L}/gu) ?? [];
  const latin = letters.filter((letter) => /\p{Script=Latin}/u.test(letter)).length;
  if (latin < MIN_LETTERS || latin < letters.length / 2) return null;

  const words = fold(stripped).match(/[a-z]+(?:'[a-z]+)*/g) ?? [];
  if (words.length < MIN_WORDS) return null;

  const hits = Object.fromEntries(LANGUAGES.map((language) => [language, 0])) as Record<
    Language,
    number
  >;
  let contractions = 0;
  let elisions = 0;
  for (const word of words) {
    if (ENGLISH_CONTRACTION.test(word)) contractions += 1;
    else if (ROMANCE_ELISION.test(word)) elisions += 1;
    const languages = LISTED.get(word) ?? [];
    for (const language of languages) hits[language] += 1 / languages.length;
  }

  return {
    polishLetters: count(stripped, POLISH_LETTERS),
    foreignLetters: count(stripped, FOREIGN_LETTERS),
    hits,
    contractions,
    elisions,
    fit: trigramFit(words),
  };
}

/* ------------------------------------------------------------ decision */

// Polish against English: the trigram likelihood ratio, the listed words of
// each, and the letters and contractions only one of them has.
const TRIGRAM_WEIGHT = 2;
const WORD_WEIGHT = 0.6;
const POLISH_LETTER_WEIGHT = 0.5;
const CONTRACTION_WEIGHT = 1.5;
/** Closer than this, the message reads as much like one as the other. */
const MIN_MARGIN = 0.4;

/**
 * The mean trigram log-likelihood at which a message reads no more like the
 * language than like any other. Most Polish and English chat scores above it
 * and most text in other languages below, but the two overlap, so it is
 * weighed against the words rather than used as a cut-off.
 */
const FIT_THRESHOLD = -7.6;
const FIT_WEIGHT = 1.5;
/** Two letters of another language outweigh a loanword; see FOREIGN_LETTERS. */
const FOREIGN_LETTERS_TO_REJECT = 2;

function decide(evidence: Evidence | null): DetectedLanguage {
  if (evidence === null) return "und";
  const { hits, polishLetters, foreignLetters, contractions, elisions, fit } = evidence;

  const polishOverEnglish =
    (fit.pl - fit.en) * TRIGRAM_WEIGHT +
    (hits.pl - hits.en) * WORD_WEIGHT +
    polishLetters * POLISH_LETTER_WEIGHT -
    contractions * CONTRACTION_WEIGHT;
  if (Math.abs(polishOverEnglish) < MIN_MARGIN) return "und";
  const candidate = polishOverEnglish > 0 ? "pl" : "en";

  if (foreignLetters >= FOREIGN_LETTERS_TO_REJECT && foreignLetters > polishLetters) return "und";

  // Whatever speaks for the winner, against the best case for any language
  // that is neither: a Czech message can beat English easily and still not be
  // Polish.
  const own =
    hits[candidate] +
    (candidate === "pl" ? polishLetters * POLISH_LETTER_WEIGHT : contractions) +
    (fit[candidate] - FIT_THRESHOLD) * FIT_WEIGHT;
  let other = 0;
  for (const language of LANGUAGES) {
    if (language !== "pl" && language !== "en") other = Math.max(other, hits[language]);
  }
  return own >= other + elisions + foreignLetters ? candidate : "und";
}

/** `pl` or `en` when the message is clearly in that language, otherwise `und`. */
export function detectLanguage(text: string): DetectedLanguage {
  return decide(evidenceOf(text));
}
