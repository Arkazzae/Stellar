import { describe, expect, it } from "vitest";

import { detectLanguage } from "./detectLanguage";

// Messages are given as the translator receives them: the markup that
// `inlineSource` writes, with code spans, links, mentions and custom emoji.
describe("chat language detection", () => {
  it.each([
    "To jest dłuższa polska wiadomość wysłana na wspólnym kanale.",
    "Ktoś wie, czemu serwer nie działa?",
    "Wczoraj wieczorem połączenie zrywało się co kilka minut.",
    "widzimy się jutro",
  ])("recognises Polish: %j", (text) => {
    expect(detectLanguage(text)).toBe("pl");
  });

  it.each([
    "nie wiem czy dam rade dzisiaj",
    "ktos wie czemu nie dziala czat?",
    "Wczoraj wieczorem polaczenie zrywalo sie co kilka minut.",
    "slychac mnie? bo cos mi trzeszczy mikrofon",
  ])("recognises Polish typed without diacritics: %j", (text) => {
    expect(detectLanguage(text)).toBe("pl");
  });

  it.each([
    "This is a longer English message posted in a shared channel.",
    "Does anyone know why the server is down?",
    "I'll post a screenshot in a sec.",
    "ngl that update broke everything",
    "see you tomorrow",
  ])("recognises English: %j", (text) => {
    expect(detectLanguage(text)).toBe("en");
  });

  it.each([
    ["Zrobiłem deploy na staging, ale build się wywalił na testach.", "pl"],
    ["Sorry, ale nie dam rady dzisiaj, mam deadline.", "pl"],
    ["mam lag na voice chacie, ktos tez?", "pl"],
    ["I'll bring pierogi to the meetup on Friday.", "en"],
    ["Dzięki, I owe you one for fixing that so fast.", "en"],
  ])("goes by the language a mixed message is built on: %j", (text, language) => {
    expect(detectLanguage(text)).toBe(language);
  });

  it.each([
    ["@marek sprawdź proszę https://github.com/stellarr/stellarr/pull/412 jak będziesz miał chwilę", "pl"],
    ["Wywala się na `npm run build`, błąd w `next.config.ts` :facepalm:", "pl"],
    ["**Uwaga:** serwer będzie niedostępny od 22:00 do 23:30 :warning:", "pl"],
    ["\\*\\*nie\\*\\* klikajcie w ten link, to phishing", "pl"],
    ["Spotkanie o 18:30, link: [tutaj](https://meet.example.com/abc-defg-hij)", "pl"],
    ["Mam 3 bilety po 120 zł na koncert 14.12, ktoś chętny?", "pl"],
    ["Napisz na jan\\@example.com, on to załatwi :thumbsup:", "pl"],
    ["Dzięki @ania.k 🙏 teraz działa", "pl"],
    ["Meeting at 18:30, link: [here](https://meet.example.com/abc-defg-hij)", "en"],
    ["Run `git pull --rebase origin main` and it should be fine", "en"],
    ["@kuba_dev @ola @tomek.w who's on call this weekend?", "en"],
    ["Error 500 on /api/v2/messages?limit=50, logs: https://logs.example.com/q/8f3a2c", "en"],
    ["Thanks @ania.k 🙏 it works now", "en"],
  ])("reads past addresses, code, mentions, emoji and numbers: %j", (text, language) => {
    expect(detectLanguage(text)).toBe(language);
  });

  // It runs on the main thread over whatever anyone typed, and a pattern that
  // backtracks across a long run of at signs takes seconds over the first two.
  it("stays fast on text made to make its patterns backtrack", () => {
    const hostile = [
      "@".repeat(2_000),
      "a@".repeat(1_000),
      "\\@".repeat(1_000),
      "`".repeat(1_000) + "a".repeat(1_000),
      "``a".repeat(700),
      "](x".repeat(700),
      "a:".repeat(1_000),
      "ą".repeat(2_000),
      "a'".repeat(1_000),
      "x".repeat(50_000),
    ];
    const started = performance.now();
    for (const text of hostile) detectLanguage(text);
    expect(performance.now() - started).toBeLessThan(250);
  });
});
