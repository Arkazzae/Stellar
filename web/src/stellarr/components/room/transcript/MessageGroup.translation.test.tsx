import { waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { translateMessage } from "@/i18n";
import { renderWithConfirm } from "@/test/confirm";

import { MessageGroup } from "./MessageGroup";

const translate = vi.hoisted(() => vi.fn(async (text: string) => `EN: ${text}`));
vi.mock("@/lib/chatTranslation/translate", () => ({
  translate,
  TranslationCancelledError: class extends Error {},
}));

const body = (id: string, text: string, encrypted?: boolean) => ({
  id,
  parts: [{ kind: "text" as const, nodes: [{ kind: "text" as const, text }] }],
  ...(encrypted ? { encrypted } : {}),
});

describe("MessageGroup translation of an encrypted message", () => {
  it("asks for its translation to be kept in memory only, and the plain one beside it to be stored", async () => {
    renderWithConfirm(
      <MessageGroup
        id="m-1"
        authorId="u-1"
        authorName="Maja"
        timestamp="2026-11-18T10:00:00Z"
        bodies={[body("m-1", "dzień dobry"), body("m-2", "login do klubu", true)]}
        translation={{
          locale: "pl",
          enabled: true,
          autoTranslate: true,
          targetLanguage: "en",
          t: (key, values) => translateMessage("pl", key, values),
        }}
      />,
    );

    await waitFor(() => expect(translate).toHaveBeenCalledTimes(2));
    expect(translate).toHaveBeenCalledWith("dzień dobry", "en", expect.objectContaining({ persist: true }));
    expect(translate).toHaveBeenCalledWith("login do klubu", "en", expect.objectContaining({ persist: false }));
  });
});
