import { describe, expect, it, vi } from "vitest";

import { encodePlaceholder, placeholderToBase64 } from "./placeholder";
import { PLACEHOLDER_URL_CACHE_LIMIT, placeholderUrlFor } from "./placeholderUrl";

const made = vi.hoisted(() => ({ count: 0 }));

vi.mock("./placeholder", async (original) => {
  const real: Record<string, unknown> = await original();
  const dataUrl = real["placeholderDataUrl"] as (bytes: Uint8Array) => string | null;
  return {
    ...real,
    placeholderDataUrl: (bytes: Uint8Array) => {
      made.count += 1;
      return dataUrl(bytes);
    },
  };
});

/** A distinct valid placeholder for each number. */
function value(n: number): string {
  const data = new Uint8ClampedArray(16).fill(255);
  data[0] = n % 256;
  data[1] = Math.floor(n / 256) % 256;
  data[4] = Math.floor(n / 65536);
  return placeholderToBase64(encodePlaceholder({ width: 2, height: 2, data }));
}

describe("placeholderUrlFor", () => {
  it("drops the least recently used value once past the limit", () => {
    const texts = new Set<string>();
    for (let n = 0; texts.size < PLACEHOLDER_URL_CACHE_LIMIT + 1; n += 1) texts.add(value(n * 37));
    const list = [...texts];

    placeholderUrlFor(list[0]);
    list.slice(1, PLACEHOLDER_URL_CACHE_LIMIT).forEach((text) => placeholderUrlFor(text));
    placeholderUrlFor(list[0]);
    const before = made.count;
    placeholderUrlFor(list[PLACEHOLDER_URL_CACHE_LIMIT]);
    expect(made.count).toBe(before + 1);

    placeholderUrlFor(list[0]);
    expect(made.count).toBe(before + 1);
    placeholderUrlFor(list[1]);
    expect(made.count).toBe(before + 2);
  });
});
