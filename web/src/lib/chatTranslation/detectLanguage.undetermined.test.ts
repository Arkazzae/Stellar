import { describe, expect, it } from "vitest";

import { detectLanguage } from "./detectLanguage";

describe("chat language detection without an answer", () => {
  it.each(["👍", "ok", "   ", "no to", "thanks!", "dzięki!"])(
    "answers und for too little text: %j",
    (text) => {
      expect(detectLanguage(text)).toBe("und");
    },
  );

  it.each([
    "hahahahahahahahahaha",
    "https://example.com/a/b/c?d=1",
    "`const x = await fetch(url)`",
    "@marek @ania.k @kuba_dev @ola",
    ":tada: :tada: :tada: :tada:",
    "12345 67890 !!! 2026-11-16",
  ])("answers und when nothing is language: %j", (text) => {
    expect(detectLanguage(text)).toBe("und");
  });

  // Czech and Slovak share much of Polish's vocabulary and, typed without
  // diacritics, most of its letters too.
  it.each([
    "Nevím, co se stalo se serverem.",
    "Mám problém s mikrofonem.",
    "ja uz jsem hotovy, a vy?",
    "v kolik se v sobotu vidime?",
    "nemam tuseni, o co mu slo",
    "Neviem, čo sa stalo so serverom.",
    "musim ist, pokecame zajtra",
    "skus to vypnut a znova zapnut",
    "nemam potuchy, o co mu islo",
    "to je divne, vcera to fungovalo",
  ])("does not take Czech or Slovak for Polish: %j", (text) => {
    expect(detectLanguage(text)).toBe("und");
  });

  it.each([
    "Ich weiß nicht, was mit dem Server passiert ist.",
    "das ist komisch, gestern ging es noch",
    "ich weiss nicht ob ich das heute noch schaffe",
    "No sé qué le pasó al servidor.",
    "ya voy, un cafe y estoy",
    "Je ne sais pas ce qui s'est passé avec le serveur.",
    "bizarre, hier ca marchait",
    "Non so cosa sia successo al server.",
    "arrivo subito, prendo un caffe",
    "Ik weet niet wat er met de server is gebeurd.",
    "raar, gisteren werkte het nog",
    "Hvala na pomoći, sad sve radi.",
    "Szia, hogy vagy?",
  ])("does not take another Latin-script language for either: %j", (text) => {
    expect(detectLanguage(text)).toBe("und");
  });

  it.each([
    "Не знаю, что случилось с сервером.",
    "Задеплоил на staging, но build опять упал.",
    "Дякую за допомогу, тепер усе працює.",
    "Δεν ξέρω τι έγινε με τον διακομιστή.",
    "لا أعرف ماذا حدث للخادم.",
    "אני לא יודע מה קרה לשרת.",
    "我不知道服务器出了什么问题。",
    "サーバーに何が起きたのか分かりません。",
    "서버에 무슨 일이 있었는지 모르겠어요.",
    "मुझे नहीं पता कि सर्वर को क्या हुआ।",
    "ไม่รู้ว่าเซิร์ฟเวอร์เป็นอะไร",
  ])("answers und for a message in another script: %j", (text) => {
    expect(detectLanguage(text)).toBe("und");
  });
});
