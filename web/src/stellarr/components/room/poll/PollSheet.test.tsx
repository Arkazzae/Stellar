import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { PollDraftRefusedError, type PollDraft } from "@/ports";
import { polish } from "@/test/translators";

import { PollSheet } from "./PollSheet";

function setup({ question = "", onSend = vi.fn(async (_draft: PollDraft) => {}) } = {}) {
  const user = userEvent.setup();
  const onClose = vi.fn();
  render(
    <PollSheet
      initialQuestion={question}
      place="Na kanale #general"
      customEmoji={[{ id: "e1", shortcode: ":dice:", glyph: "/emoji/dice.png" }]}
      onSend={onSend}
      onClose={onClose}
      t={polish}
    />,
  );
  const sheet = screen.getByRole("dialog", { name: "Nowa ankieta" });
  const answer = (number: number) => within(sheet).getByRole("textbox", { name: `Odpowiedź ${number}` });
  const send = within(sheet).getByRole("button", { name: "Wyślij ankietę" });
  return { user, sheet, answer, send, onSend, onClose };
}

describe("the new poll sheet", () => {
  it("starts with the question typed after the command, and counts what it may hold", () => {
    const { sheet } = setup({ question: "Kiedy gramy w ten weekend?" });

    expect(within(sheet).getByRole("textbox", { name: /Pytanie/ })).toHaveValue("Kiedy gramy w ten weekend?");
    expect(within(sheet).getByText("26 / 300")).toBeInTheDocument();
    expect(within(sheet).getByText("2 z 10")).toBeInTheDocument();
    expect(within(sheet).getByText("Na kanale #general")).toBeInTheDocument();
  });

  it("sends only with a question and two answers, leaving empty answers out, with a day and one answer each", async () => {
    const { user, sheet, answer, send, onSend, onClose } = setup({ question: "Kiedy?" });
    expect(send).toBeDisabled();

    await user.type(answer(1), "Piątek");
    await user.click(within(sheet).getByRole("button", { name: "Dodaj odpowiedź" }));
    await user.type(answer(3), "Sobota");
    expect(send).toBeEnabled();
    await user.click(send);

    expect(onSend).toHaveBeenCalledExactlyOnceWith({
      question: "Kiedy?",
      answers: [{ text: "Piątek" }, { text: "Sobota" }],
      multiple: false,
      duration: "1d",
    });
    expect(onClose).toHaveBeenCalledOnce();
  });

  it("holds ten answers at most and two at least, each up to 55 characters", async () => {
    const { user, sheet, answer } = setup();
    expect(within(sheet).queryByRole("button", { name: "Usuń odpowiedź" })).toBeNull();
    expect(answer(1)).toHaveAttribute("maxLength", "55");
    expect(within(sheet).getByRole("textbox", { name: /Pytanie/ })).toHaveAttribute("maxLength", "300");

    for (let added = 0; added < 8; added++) {
      await user.click(within(sheet).getByRole("button", { name: "Dodaj odpowiedź" }));
    }

    expect(within(sheet).getByText("10 z 10")).toBeInTheDocument();
    expect(within(sheet).queryByRole("button", { name: "Dodaj odpowiedź" })).toBeNull();
    expect(within(sheet).getAllByRole("button", { name: "Usuń odpowiedź" })).toHaveLength(10);
  });

  it("moves an answer with the arrow keys on its handle", async () => {
    const { user, sheet, answer, send, onSend } = setup({ question: "Kiedy?" });
    await user.type(answer(1), "Piątek");
    await user.type(answer(2), "Sobota");

    within(sheet).getAllByRole("button", { name: "Przesuń odpowiedź" })[1]?.focus();
    await user.keyboard("{ArrowUp}");
    await user.click(send);

    expect(onSend.mock.calls[0]?.[0].answers).toEqual([{ text: "Sobota" }, { text: "Piątek" }]);
  });

  it("gives an answer an emoji from the picker, and Escape closes only the picker", async () => {
    const { user, sheet, answer, send, onSend } = setup({ question: "Kiedy?" });
    await user.type(answer(1), "Piątek");
    await user.type(answer(2), "Sobota");

    await user.click(within(sheet).getAllByRole("button", { name: "Dodaj emoji" })[0]!);
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog", { name: "Emoji odpowiedzi" })).toBeNull();
    expect(screen.getByRole("dialog", { name: "Nowa ankieta" })).toBeInTheDocument();

    await user.click(within(sheet).getAllByRole("button", { name: "Dodaj emoji" })[0]!);
    await user.click(within(screen.getByRole("dialog", { name: "Emoji odpowiedzi" })).getByRole("option", { name: ":dice:" }));
    await user.click(send);

    expect(onSend.mock.calls[0]?.[0].answers[0]).toEqual({ text: "Piątek", emoji: ":dice:" });
  });

  it("takes several answers and a longer time when asked", async () => {
    const { user, sheet, answer, send, onSend } = setup({ question: "Co bierzemy?" });
    await user.type(answer(1), "Planszówki");
    await user.type(answer(2), "Konsola");

    await user.click(within(sheet).getByRole("switch", { name: "Kilka odpowiedzi" }));
    await user.click(within(sheet).getByRole("combobox", { name: "Czas na głosowanie" }));
    await user.click(screen.getByRole("option", { name: "3 dni" }));
    await user.click(send);

    expect(onSend.mock.calls[0]?.[0]).toMatchObject({ multiple: true, duration: "3d" });
  });

  it("marks what the server refused, by the answer it was sent as", async () => {
    const onSend = vi.fn(async () => Promise.reject(new PollDraftRefusedError([{ answer: 1 }])));
    const { user, sheet, answer, send, onClose } = setup({ question: "Kiedy?", onSend });
    await user.type(answer(1), "Piątek");
    await user.click(within(sheet).getByRole("button", { name: "Dodaj odpowiedź" }));
    await user.type(answer(3), "Sobota");

    await user.click(send);

    // The second answer sent was the third field: the empty one was left out.
    expect(await within(sheet).findByText(/Tej odpowiedzi serwer nie przyjął/)).toBeInTheDocument();
    expect(answer(3)).toHaveAttribute("aria-invalid", "true");
    expect(answer(1)).not.toHaveAttribute("aria-invalid");
    expect(onClose).not.toHaveBeenCalled();
  });
});
