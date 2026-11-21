import { describe, expect, it } from "vitest";

import { translateMessage, type Translator } from "@/i18n";

import {
  banPerson,
  channelSubject,
  messageSubject,
  personSubject,
  sectionSubject,
  serverSubject,
} from "./confirmations";

const pl: Translator = (key, values) => translateMessage("pl", key, values);
const en: Translator = (key, values) => translateMessage("en", key, values);

describe("confirmation subjects", () => {
  it("says what a channel is and where it sits", () => {
    expect(channelSubject(pl, { kind: "text", name: "announcements", sectionName: "Information" })).toEqual({
      kind: "channel",
      channelKind: "text",
      name: "announcements",
      detail: "Kanał tekstowy w sekcji Information",
    });
    expect(channelSubject(pl, { kind: "voice", name: "voice" }).detail).toBe("Kanał głosowy");
    expect(channelSubject(en, { kind: "voice", name: "voice", sectionName: "Games" }).detail).toBe(
      "Voice channel in Games",
    );
  });

  it("counts a server's people and channels with the plural each number takes, when they are known", () => {
    expect(serverSubject(pl, "pl", { name: "Stellarr Dev", mark: "SD", memberCount: 29, channelCount: 17 }).detail).toBe(
      "29 osób · 17 kanałów",
    );
    expect(serverSubject(pl, "pl", { name: "Mały", mark: "MA", memberCount: 2, channelCount: 1 }).detail).toBe(
      "2 osoby · 1 kanał",
    );
    expect(serverSubject(pl, "pl", { name: "Mały", mark: "MA" })).not.toHaveProperty("detail");
  });

  it("gives a person's handle only when there is one", () => {
    expect(personSubject({ id: "u1", displayName: "Ania Mazur", username: "ania" })).toEqual({
      kind: "person",
      id: "u1",
      name: "Ania Mazur",
      handle: "ania",
    });
    expect(personSubject({ id: "u1", displayName: "Ania Mazur" })).not.toHaveProperty("handle");
  });

  it("says so for a message with no words", () => {
    const subject = messageSubject(pl, { authorId: "u1", authorName: "Kenji", text: "  " });
    expect(subject).toMatchObject({ kind: "message", text: "Wiadomość bez tekstu" });
  });

  it("calls a section empty only when its channels are known", () => {
    expect(sectionSubject(pl, { name: "Information", channels: [] })).toMatchObject({
      channels: [],
      emptyDetail: "Bez kanałów",
    });
    expect(sectionSubject(pl, { name: "Information", channels: null })).not.toHaveProperty("emptyDetail");
  });
});

describe("the ban question", () => {
  it("asks for a reason in sentence case without requiring one", () => {
    const request = banPerson(pl, personSubject({ id: "u1", displayName: "Jo" }));

    expect(request.title).toBe("Zbanować tę osobę?");
    expect(request.field).toMatchObject({ label: "Powód", optional: true });
    expect(banPerson(en, personSubject({ id: "u1", displayName: "Jo" })).field.label).toBe("Reason");
  });
});
