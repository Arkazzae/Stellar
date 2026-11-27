import { describe, expect, it } from "vitest";

import de from "../../../../../.locales/web/de.json";
import en from "../../../../../.locales/web/en.json";
import pl from "../../../../../.locales/web/pl.json";

import { COMMANDS } from "./commandRegistry";

describe("the command names in every language pack", () => {
  it("are single lower-case words, each answering to one command alongside the English ones", () => {
    for (const pack of [pl, en, de] as Record<string, string>[]) {
      const owners = new Map<string, string>();
      for (const command of COMMANDS) {
        const name = pack[command.name];
        expect(name, command.name).toMatch(/^\p{Ll}+$/u);
        for (const word of new Set([name, command.alias])) {
          expect(owners.get(word ?? ""), `${word} names two commands`).toBeUndefined();
          owners.set(word ?? "", command.id);
        }
      }
    }
  });
});
