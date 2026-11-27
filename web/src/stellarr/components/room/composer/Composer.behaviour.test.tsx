import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { Composer, type ComposerProps } from "./Composer";

const baseProps: ComposerProps = {
  channelName: "general",
  chips: [],
  onSubmit: vi.fn(),
  onRemoveChip: vi.fn(),
  onRetryChip: vi.fn(),
  onReorderChips: vi.fn(),
};

function setup(props: Partial<ComposerProps> = {}) {
  const user = userEvent.setup();
  const view = render(<Composer {...baseProps} {...props} />);
  return { user, ...view, field: screen.getByRole("combobox") as HTMLTextAreaElement };
}

describe("Composer replying", () => {
  it("names who is being answered, focuses the field and lets the reply be dropped", async () => {
    const onCancelReply = vi.fn();
    const { user, field } = setup({
      replyingTo: { id: "m-1", authorName: "Ada", preview: "see you at nine" },
      onCancelReply,
    });

    expect(screen.getByText("Ada")).toBeInTheDocument();
    expect(screen.getByText("see you at nine")).toBeInTheDocument();
    expect(field).toHaveFocus();

    await user.click(screen.getByRole("button", { name: "Anuluj odpowiedź" }));
    expect(onCancelReply).toHaveBeenCalledOnce();
  });

  it("does not take focus back while the same message stays being answered", () => {
    const { field, rerender } = setup({
      replyingTo: { id: "m-1", authorName: "Ada", preview: "" },
    });
    const elsewhere = screen.getByRole("button", { name: "Formatowanie" });
    elsewhere.focus();

    // The host rebuilds the object on every render; the message is the same.
    rerender(
      <Composer {...baseProps} replyingTo={{ id: "m-1", authorName: "Ada", preview: "" }} />,
    );

    expect(field).not.toHaveFocus();
    expect(elsewhere).toHaveFocus();
  });
});

describe("Composer sending", () => {
  it("keeps the draft and says so when the message could not be sent", async () => {
    const onSubmit = vi.fn().mockRejectedValue(new Error("offline"));
    const { user, field } = setup({ onSubmit });

    await user.type(field, "ważne");
    await user.click(screen.getByRole("button", { name: "Wyślij" }));

    expect(await screen.findByRole("alert")).toHaveTextContent("Nie udało się wysłać wiadomości");
    expect(field).toHaveValue("ważne");

    await user.type(field, "!");
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("refuses a code block that grows past the message limit once it is fenced", async () => {
    const onSubmit = vi.fn();
    const { user, field } = setup({ onSubmit });

    await user.click(screen.getByRole("button", { name: "Wyślij jako kod" }));
    const draft = "x".repeat(99_995);
    fireEvent.change(field, { target: { value: draft } });
    await user.click(screen.getByRole("button", { name: "Wyślij" }));

    expect(screen.getByRole("alert")).toHaveTextContent(/Blok kodu może mieć maksymalnie 100\s000 znaków/);
    expect(onSubmit).not.toHaveBeenCalled();
    expect(field).toHaveValue(draft);
  });

  it("turns a long paste into a code message and preserves literal fence lines", async () => {
    const onSubmit = vi.fn();
    const { user, field } = setup({ onSubmit });
    const draft = `  ${"x".repeat(21_000)}\n\`\`\`\n@ada\n`;

    await user.click(field);
    await user.paste(draft);

    expect(field).toHaveFocus();
    expect(field).toHaveValue(draft);
    expect(screen.getByRole("combobox", { name: "Język kodu wiadomości" })).toBeInTheDocument();
    await user.keyboard("{Control>}{Enter}{/Control}");
    expect(onSubmit).toHaveBeenCalledWith(`\`\`\`\`plaintext\n${draft}\n\`\`\`\``);
  });

  it("sends a message on Enter but breaks the line on Shift+Enter", async () => {
    const onSubmit = vi.fn();
    const { user, field } = setup({ onSubmit });

    await user.type(field, "pierwsza{Shift>}{Enter}{/Shift}druga");
    expect(onSubmit).not.toHaveBeenCalled();
    await user.keyboard("{Enter}");

    expect(onSubmit).toHaveBeenCalledWith("pierwsza\ndruga");
  });
});

describe("Composer attachments", () => {
  it("offers no file and says why the controls that remain are off when nothing can be uploaded", async () => {
    const { user, field } = setup({
      attachmentsSupported: false,
      onAttach: vi.fn(),
      openVoiceMessageMicrophone: vi.fn(),
      onSendVoice: vi.fn(),
    });

    await user.type(field, "/");
    expect(screen.getByRole("option", { name: /^\/kod/ })).toBeInTheDocument();
    expect(screen.queryByRole("option", { name: /^\/plik/ })).toBeNull();
    expect(screen.getByRole("button", { name: "Nagraj wiadomość głosową" })).toBeDisabled();

    await user.keyboard("{Escape}");
    await user.clear(field);
    await user.click(screen.getByRole("button", { name: "Wyślij jako kod" }));
    const reason = "Załączniki w wiadomościach prywatnych nie są jeszcze dostępne";
    expect(screen.getByRole("button", { name: "Dodaj załącznik" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Dodaj załącznik" })).toHaveAttribute("title", reason);
  });

  it("stops accepting files at ten, and says so beside /plik", async () => {
    const chips = Array.from({ length: 10 }, (_, index) => ({
      id: `c-${index}`,
      name: `file-${index}.txt`,
      status: "ready" as const,
    }));
    const { user, field, container } = setup({ chips, onAttach: vi.fn() });
    const click = vi.spyOn(container.querySelector<HTMLInputElement>('input[type="file"]')!, "click");

    await user.type(field, "/plik");
    const row = screen.getByRole("option", { name: /plik/ });
    expect(row).toHaveAttribute("aria-disabled", "true");
    expect(screen.getByText("Wiadomość może zawierać maksymalnie 10 załączników")).toBeInTheDocument();
    await user.keyboard("{Enter}");
    expect(click).not.toHaveBeenCalled();

    await user.keyboard("{Escape}");
    await user.clear(field);
    await user.click(screen.getByRole("button", { name: "Wyślij jako kod" }));
    expect(screen.getByRole("button", { name: "Dodaj załącznik" })).toBeDisabled();
  });

  it("ignores a paste of plain text, leaving it to the field", () => {
    const onAttach = vi.fn();
    setup({ onAttach });

    fireEvent.paste(screen.getByRole("combobox"), { clipboardData: { files: [] } });

    expect(onAttach).not.toHaveBeenCalled();
  });
});

describe("Composer popovers", () => {
  it("closes the emoji picker on Escape and returns focus to its button", async () => {
    const { user } = setup();

    await user.click(screen.getByRole("button", { name: "Emoji" }));
    expect(screen.getByRole("dialog", { name: "Szukaj emoji" })).toBeInTheDocument();
    await user.keyboard("{Escape}");

    expect(screen.queryByRole("dialog", { name: "Szukaj emoji" })).toBeNull();
    expect(screen.getByRole("button", { name: "Emoji" })).toHaveFocus();
  });

  it("closes the formatting bar on a press elsewhere, but not on one inside it", async () => {
    const { user } = setup();

    await user.click(screen.getByRole("button", { name: "Formatowanie" }));
    fireEvent.pointerDown(screen.getByRole("button", { name: "Kursywa" }));
    expect(screen.getByRole("toolbar", { name: "Formatowanie" })).toBeInTheDocument();

    await user.click(document.body);
    expect(screen.queryByRole("toolbar", { name: "Formatowanie" })).toBeNull();
  });

  it("folds code mode's mobile tray when a press lands outside it", async () => {
    const { user } = setup();
    await user.click(screen.getByRole("button", { name: "Wyślij jako kod" }));
    const more = screen.getByRole("button", { name: "Więcej opcji wiadomości" });

    await user.click(more);
    expect(more).toHaveAttribute("aria-expanded", "true");
    await user.click(document.body);

    expect(more).toHaveAttribute("aria-expanded", "false");
  });

  describe("with the command list opened over the picker from the keyboard", () => {
    const commands = () => screen.queryByRole("listbox", { name: "Komendy" });

    async function openListOverPicker() {
      const { user } = setup();
      const plus = screen.getByRole("button", { name: "Komendy i dodawanie" });
      await user.click(screen.getByRole("button", { name: "Emoji" }));
      // The keyboard reaches the plus and presses it: no pointer is involved,
      // so the picker stays open under the list.
      plus.focus();
      fireEvent.click(plus, { detail: 0 });
      expect(plus).toHaveAttribute("aria-expanded", "true");
      expect(commands()).toBeInTheDocument();
      expect(screen.getByRole("dialog", { name: "Szukaj emoji" })).toBeInTheDocument();
      return { user, plus };
    }

    it("closes the list on one Escape, handing the focus to the field, and the picker on the next", async () => {
      const { user } = await openListOverPicker();

      await user.keyboard("{Escape}");
      expect(commands()).toBeNull();
      expect(screen.getByRole("combobox")).toHaveFocus();
      expect(screen.getByRole("dialog", { name: "Szukaj emoji" })).toBeInTheDocument();

      await user.keyboard("{Escape}");
      expect(screen.queryByRole("dialog", { name: "Szukaj emoji" })).toBeNull();
    });

    it("closes the list on a press outside both and the picker on the next", async () => {
      const { user } = await openListOverPicker();

      await user.click(document.body);
      expect(commands()).toBeNull();
      expect(screen.getByRole("dialog", { name: "Szukaj emoji" })).toBeInTheDocument();

      await user.click(document.body);
      expect(screen.queryByRole("dialog", { name: "Szukaj emoji" })).toBeNull();
    });

    it("closes the list, and leaves the picker open, on a press inside the picker", async () => {
      await openListOverPicker();

      fireEvent.pointerDown(
        screen.getByRole("dialog", { name: "Szukaj emoji" }).querySelector(".emo-pick")!,
      );
      expect(commands()).toBeNull();
      expect(screen.getByRole("dialog", { name: "Szukaj emoji" })).toBeInTheDocument();
    });
  });
});
