import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { ConfirmProvider } from "@/components/primitives/ConfirmDialog";
import type { MessageTranslationOptions } from "@/components/room/attachments/TranslatedTextAttachment";
import type { MessageGroupVM } from "@/components/room/transcript/MessageGroup";
import type { TranscriptSectionVM } from "@/components/room/transcript/Transcript";
import { translateMessage } from "@/i18n";
import { fakeTranscriptLayout } from "@/test/transcriptLayout";

import { DmChat, type DmChatProps } from "./DmChat";

const ada = { id: "u-ada", displayName: "Ada", initials: "AD", presence: "online" } as const;

function message(id: string, authorId: string, authorName: string, text: string): MessageGroupVM {
  return {
    id,
    authorId,
    authorName,
    timestamp: "2026-08-14T10:00:00Z",
    bodies: [{ id, parts: [{ kind: "text", nodes: [{ kind: "text", text }] }] }],
  };
}

function openChat(overrides: Partial<DmChatProps> = {}) {
  const props: DmChatProps = {
    peer: ada,
    username: "ada",
    sections: [],
    onSend: vi.fn().mockResolvedValue(undefined),
    chips: [],
    onAttach: vi.fn(),
    onRemoveChip: vi.fn(),
    onRetryChip: vi.fn(),
    onReorderChips: vi.fn(),
    onOpenProfile: vi.fn(),
    ...overrides,
  };
  // Message groups ask for confirmation before deleting; the app wraps them.
  const view = render(<DmChat {...props} />, { wrapper: ConfirmProvider });
  return { props, user: userEvent.setup(), ...view };
}

describe("DmChat history", () => {
  it("begins with whose conversation it is and who can read it", () => {
    openChat({ introAction: <button type="button">Działanie</button> });

    const log = screen.getByRole("log", { name: "Wiadomości z Ada" });
    expect(within(log).getByRole("heading", { level: 3, name: "Ada" })).toBeTruthy();
    expect(within(log).getByText("@ada")).toBeTruthy();
    expect(
      within(log).getByText(
        "To początek Waszej rozmowy. Wiadomości są zapisane na serwerze bez szyfrowania, więc ma do nich dostęp jego administrator.",
      ),
    ).toBeTruthy();
    expect(within(log).getByRole("button", { name: "Działanie" })).toBeTruthy();
  });

  it("shows the messages and opens the author's card from them", async () => {
    const sections: TranscriptSectionVM[] = [
      { day: "Dzisiaj", groups: [message("m1", ada.id, "Ada", "Cześć, jesteś?")] },
    ];
    const { user, props } = openChat({ sections });

    expect(screen.getByText("Cześć, jesteś?")).toBeTruthy();
    await user.click(screen.getByRole("button", { name: "Profil: Ada" }));

    expect(props.onOpenProfile).toHaveBeenCalledWith(ada.id, expect.any(HTMLElement));
  });

  it("asks for older messages when the reader reaches the top, and not while loading them", () => {
    const layout = fakeTranscriptLayout();
    const onLoadOlderMessages = vi.fn();
    const sections: TranscriptSectionVM[] = [
      {
        day: "Dzisiaj",
        groups: Array.from({ length: 20 }, (_, index) =>
          message(`m${index}`, ada.id, "Ada", `Wiadomość ${index}`),
        ),
      },
    ];
    const { rerender, props } = openChat({
      sections,
      hasOlderMessages: true,
      loadingOlderMessages: true,
      onLoadOlderMessages,
    });

    layout.scrollTo(0);
    expect(onLoadOlderMessages).not.toHaveBeenCalled();

    rerender(<DmChat {...props} loadingOlderMessages={false} />);
    expect(onLoadOlderMessages).toHaveBeenCalledOnce();
  });

  it("does not ask for older messages when the history has none", () => {
    const layout = fakeTranscriptLayout();
    const onLoadOlderMessages = vi.fn();
    openChat({ hasOlderMessages: false, onLoadOlderMessages });

    layout.scrollTo(0);

    expect(onLoadOlderMessages).not.toHaveBeenCalled();
  });
});

describe("DmChat composer", () => {
  it("is addressed to the person", () => {
    openChat();

    expect(screen.getByRole("combobox", { name: "Napisz do @Ada" })).toHaveAttribute(
      "placeholder",
      "Napisz do @Ada",
    );
  });

  it("speaks English when the translator it is given does", () => {
    const translation: MessageTranslationOptions = {
      locale: "en",
      enabled: false,
      autoTranslate: false,
      targetLanguage: "en",
      t: (key, values) => translateMessage("en", key, values),
    };
    openChat({ translation });

    expect(screen.getByRole("combobox", { name: "Message @Ada" })).toBeTruthy();
    expect(screen.getByRole("log", { name: "Messages with Ada" })).toBeTruthy();
  });

  it("sends the trimmed text on Enter", async () => {
    const { user, props } = openChat();

    await user.type(screen.getByRole("combobox"), "  do zobaczenia  {Enter}");

    expect(props.onSend).toHaveBeenCalledExactlyOnceWith("do zobaczenia");
  });

  it("sends nothing for an empty message", async () => {
    const { user, props } = openChat();

    await user.type(screen.getByRole("combobox"), "   {Enter}");

    expect(props.onSend).not.toHaveBeenCalled();
  });

  it("hands over the files that are attached", async () => {
    const { user, props, container } = openChat();
    const file = new File(["x"], "plan.pdf", { type: "application/pdf" });

    await user.upload(container.querySelector<HTMLInputElement>('input[type="file"]')!, file);

    expect(props.onAttach).toHaveBeenCalledWith([file]);
  });
});
