import { readdirSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

import { defaultIdentityArt, hueFor } from "@/design/identity";

const here = (path: string) => fileURLToPath(new URL(path, import.meta.url));

/** The files the export script wrote, by the address the app asks for. */
function served(folder: "avatars" | "banners"): string[] {
  return readdirSync(join(here("../../../public/identity"), folder))
    .filter((file) => file.endsWith(".webp"))
    .map((file) => `/identity/${folder}/${file}`)
    .sort();
}

const people = Array.from({ length: 5_000 }, (_, index) => {
  const id = `person-${index}`;
  return { ...defaultIdentityArt(id), hue: hueFor(id) };
});

function distinct(key: (person: (typeof people)[number]) => string): number {
  return new Set(people.map(key)).size;
}

describe("default identity artwork", () => {
  it("keeps a person on the same face and banner", () => {
    expect(defaultIdentityArt("person-42")).toEqual({
      avatarUrl: "/identity/avatars/03-skip-rabbit.webp",
      bannerUrl: "/identity/banners/13-observatory.webp",
    });
  });

  it("picks among exactly the exported files and reaches every one", () => {
    expect([...new Set(people.map((p) => p.avatarUrl))].sort()).toEqual(
      served("avatars"),
    );
    expect([...new Set(people.map((p) => p.bannerUrl))].sort()).toEqual(
      served("banners"),
    );
  });

  it("picks the face, the banner and the hue apart from each other", () => {
    const faces = served("avatars").length;
    const banners = served("banners").length;

    expect(distinct((p) => `${p.avatarUrl} ${p.bannerUrl}`)).toBe(faces * banners);
    expect(distinct((p) => `${p.avatarUrl} ${p.hue}`)).toBe(faces * 12);
    expect(distinct((p) => `${p.bannerUrl} ${p.hue}`)).toBe(banners * 12);
  });
});
