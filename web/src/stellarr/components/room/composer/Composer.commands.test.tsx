import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type * as GiphyModule from "@/lib/giphy/giphy";

import { Composer, type ComposerProps, type MentionCandidate } from "./Composer";

const giphy = vi.hoisted(() => ({ fetchPage: vi.fn(), resolveApiKey: vi.fn() }));

vi.mock("@/lib/giphy/giphy", async (importOriginal) => ({
  ...(await importOriginal<typeof GiphyModule>()),
  fetchGiphyPage: giphy.fetchPage,
  resolveGiphyApiKey: giphy.resolveApiKey,
  trackGiphyAction: vi.fn(),
}));

beforeEach(() => {
  giphy.resolveApiKey.mockResolvedValue("test-key");
  giphy.fetchPage.mockResolvedValue({ items: [], offset: 0, count: 0, totalCount: 0 });
});

const ADA: MentionCandidate = { id: "u-ada", username: "ada", displayName: "Ada Lovelace" };
const BOB: MentionCandidate = { id: "u-bob", username: "bob", displayName: "Bob Stone" };

function setup(props: Partial<ComposerProps> = {}) {
  const user = userEvent.setup();
  const onSubmit = vi.fn();
  const view = render(
    <Composer
      channelName="general"
      chips={[]}
      onSubmit={onSubmit}
      onRemoveChip={vi.fn()}
      onRetryChip={vi.fn()}
      onReorderChips={vi.fn()}
      {...props}
    />,
  );
  const field = screen.getByRole("combobox", { name: "Napisz na #general" }) as HTMLTextAreaElement;
  return { user, onSubmit, field, ...view };
}

const commandList = () => screen.queryByRole("listbox", { name: "Komendy" });
/** The command names as listed, in order. */
const listed = () =>
  within(commandList()!)
    .getAllByRole("option")
    .map((option) => option.textContent?.trim().split(" ")[0]);
/** A command waits a frame for its field to be back before it moves the focus. */
const nextFrame = () =>
  act(() => new Promise<void>((resolve) => requestAnimationFrame(() => resolve())));

describe("Composer command list", () => {
  it("opens from the plus in groups, with a hint that follows the selection", async () => {
    const { user, field } = setup({
      onAttach: vi.fn(),
      commandHost: { openPinned: vi.fn(), openStatus: vi.fn() },
    });

    const plus = screen.getByRole("button", { name: "Komendy i dodawanie" });
    await user.click(plus);
    await nextFrame();

    // Driven from the field, which keeps the focus, as the list after "@" is.
    expect(field).toHaveFocus();
    expect(plus).toHaveAttribute("aria-expanded", "true");
    const list = commandList()!;
    const group = (name: string) =>
      within(within(list).getByRole("group", { name }))
        .getAllByRole("option")
        .map((option) => option.textContent?.trim());
    expect(group("Dodaj")).toEqual([expect.stringMatching(/^\/plik/)]);
    expect(group("Wiadomość")).toEqual([expect.stringMatching(/^\/kod/), expect.stringMatching(/^\/spoiler/)]);
    expect(group("Serwer")).toEqual([
      expect.stringMatching(/^\/przypięte/),
      expect.stringMatching(/^\/status/),
    ]);

    const selected = () => within(list).getByRole("option", { selected: true });
    expect(field).toHaveAttribute("aria-activedescendant", selected().id);
    expect(selected()).toHaveAccessibleDescription(/Otwiera wybór plików/);

    await user.keyboard("{ArrowDown}");
    expect(selected()).toHaveTextContent(/^\/kod/);
    expect(field).toHaveAttribute("aria-activedescendant", selected().id);
    // The hint says what it takes, marking what may be left out, and how it looks.
    expect(selected()).toHaveAccessibleDescription(/Przełącza pole w tryb kodu/);
    expect(selected()).toHaveAccessibleDescription(/Opcjonalnie\. Na przykład ts, python albo rust/);
    expect(selected()).toHaveAccessibleDescription(/\/kod python/);

    await user.keyboard("{Escape}");
    expect(commandList()).toBeNull();
    expect(field).toHaveFocus();
  });

  it("narrows to what the typed word finds, without diacritics and by the English name", async () => {
    const { user, field } = setup({
      openVoiceMessageMicrophone: vi.fn(),
      onSendVoice: vi.fn(),
      commandHost: { kick: vi.fn(), ban: vi.fn() },
    });

    await user.type(field, "/glos");
    expect(listed()).toEqual(["/głosówka"]);
    expect(within(commandList()!).getByText("głos", { selector: "mark" })).toBeInTheDocument();

    await user.clear(field);
    await user.type(field, "/kick");
    // The reader's own name, with the English one beside it marked.
    expect(listed()).toEqual(["/wyrzuć"]);
    expect(within(commandList()!).getByText("kick", { selector: "mark" })).toBeInTheDocument();

    await user.clear(field);
    await user.type(field, "/za");
    expect(listed()).toEqual(["/zbanuj"]);
  });

  it("opens nothing for a slash inside a sentence, and sends a slash word that finds nothing as typed", async () => {
    const { user, field, onSubmit } = setup();

    await user.type(field, "hej /gif");
    expect(commandList()).toBeNull();
    await user.keyboard("{Enter}");
    expect(onSubmit).toHaveBeenLastCalledWith("hej /gif");

    await user.clear(field);
    await user.type(field, "/nieznane");
    expect(commandList()).toBeNull();
    await user.keyboard("{Enter}");
    expect(onSubmit).toHaveBeenLastCalledWith("/nieznane");

    // A word put away with Escape is text, and so is the space after it.
    await user.clear(field);
    await user.type(field, "/kod{Escape} działa");
    expect(screen.queryByRole("textbox", { name: "język" })).toBeNull();
    await user.keyboard("{Enter}");
    expect(onSubmit).toHaveBeenLastCalledWith("/kod działa");
  });

  it("leaves a slash to the code in code mode, where a comment starts with two", async () => {
    const { user } = setup();

    await user.click(screen.getByRole("button", { name: "Wyślij jako kod" }));
    const code = screen.getByRole("textbox", { name: "Napisz na #general" });
    await user.type(code, "// komentarz");

    expect(commandList()).toBeNull();
    expect(code).toHaveValue("// komentarz");
  });

  it("lists only what the place has and the person may do", async () => {
    const { user, field, unmount } = setup();

    await user.type(field, "/");
    // Without files, GIFs, a recorder or a host, the message's own two are left.
    expect(listed()).toEqual(["/kod", "/spoiler"]);
    unmount();

    const second = setup({ commandHost: { kick: vi.fn(), people: [ADA] } });
    await second.user.type(second.field, "/");
    expect(listed()).toContain("/wyrzuć");
    expect(listed()).not.toContain("/zbanuj");
    expect(listed()).not.toContain("/przypięte");
  });
});

describe("Composer commands", () => {
  it("starts the voice recorder from /głosówka, with nothing of the command left in the draft", async () => {
    const { user, field } = setup({
      openVoiceMessageMicrophone: vi.fn(() => new Promise<never>(() => {})),
      onSendVoice: vi.fn(),
    });

    await user.type(field, "/glos{Enter}");

    // The recorder takes the field's place.
    expect(screen.getByRole("button", { name: "Anuluj wiadomość głosową" })).toBeInTheDocument();
    expect(field).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Anuluj wiadomość głosową" }));
    expect(screen.getByRole("combobox", { name: "Napisz na #general" })).toHaveValue("");
  });

  it("opens the GIF browser with the words given to /gif", async () => {
    const { user, field } = setup({ onGif: vi.fn().mockResolvedValue(undefined) });

    await user.type(field, "/gif{Enter}");
    const words = screen.getByRole("textbox", { name: "szukaj" });
    expect(words).toHaveFocus();
    await user.type(words, "kot{Enter}");

    expect(screen.getByRole("dialog", { name: "Przeglądarka GIF-ów" })).toBeVisible();
    expect(screen.getByRole("searchbox")).toHaveValue("kot");
    await waitFor(() =>
      expect(giphy.fetchPage).toHaveBeenCalledWith(expect.objectContaining({ query: "kot" })),
    );
  });

  it("enters code mode with the language given to /kod", async () => {
    const { user, field, onSubmit } = setup();

    // A space after the whole name takes the command up, as choosing it does.
    await user.type(field, "/kod ");
    await user.type(screen.getByRole("textbox", { name: "język" }), "ts{Enter}");
    await nextFrame();

    const code = screen.getByRole("textbox", { name: "Napisz na #general" });
    await user.type(code, "let a = 1");
    await user.keyboard("{Control>}{Enter}{/Control}");
    expect(onSubmit).toHaveBeenCalledWith("```typescript\nlet a = 1\n```");
  });

  it("puts the words given to /spoiler in the draft as a spoiler, to be sent like any text", async () => {
    const { user, field, onSubmit } = setup();

    await user.type(field, "/spoiler{Enter}");
    await user.type(screen.getByRole("textbox", { name: "tekst" }), "koniec filmu{Enter}");
    await nextFrame();

    expect(field).toHaveValue("||koniec filmu|| ");
    expect(field).toHaveFocus();
    await user.keyboard("{Enter}");
    expect(onSubmit).toHaveBeenCalledWith("||koniec filmu||");
  });

  it("opens the pinned list and the account panel through the host", async () => {
    const openPinned = vi.fn();
    const openStatus = vi.fn();
    const { user, field } = setup({ commandHost: { openPinned, openStatus } });

    await user.type(field, "/przypi{Enter}");
    expect(openPinned).toHaveBeenCalledTimes(1);
    expect(field).toHaveValue("");
    // The caret goes back into the field a frame later.
    await nextFrame();

    await user.type(field, "/status{Enter}");
    expect(openStatus).toHaveBeenCalledTimes(1);
    expect(openPinned).toHaveBeenCalledTimes(1);
  });

  it("asks the host to ban only with a picked person, passing the reason typed after them", async () => {
    let focusedWhenAsked: Element | null = null;
    const ban = vi.fn(() => {
      focusedWhenAsked = document.activeElement;
    });
    const { user, field } = setup({ commandHost: { ban, people: [ADA, BOB] } });

    await user.type(field, "/zbanuj ");
    const person = screen.getByRole("combobox", { name: "osoba" });
    expect(person).toHaveFocus();
    // Nobody picked yet: the forward button waits, and Enter has nothing to run.
    expect(screen.getByRole("button", { name: "Dalej" })).toBeDisabled();
    expect(within(screen.getByRole("listbox", { name: "Oznacz osobę lub bota" })).getAllByRole("option"))
      .toHaveLength(2);

    // Tab takes the highlighted person and moves on to the reason.
    await user.type(person, "bo");
    await user.keyboard("{Tab}");
    expect(person).toHaveValue("Bob Stone");
    const reason = screen.getByRole("textbox", { name: "powód" });
    expect(reason).toHaveFocus();

    await user.type(reason, "spam{Enter}");
    await nextFrame();

    expect(ban).toHaveBeenCalledExactlyOnceWith(BOB, "spam");
    // The field had the focus back when the host was asked, so the question
    // card hands it back there.
    expect(focusedWhenAsked).toBe(field);
    expect(field).toHaveValue("");
  });

  it("asks the host to kick the person picked after the English name", async () => {
    const kick = vi.fn();
    const { user, field } = setup({ commandHost: { kick, people: [ADA, BOB] } });

    await user.type(field, "/kick ");
    expect(screen.getByRole("group", { name: "Komenda /wyrzuć" })).toBeInTheDocument();
    await user.type(screen.getByRole("combobox", { name: "osoba" }), "ad{Enter}");
    await user.keyboard("{Enter}");
    await nextFrame();

    expect(kick).toHaveBeenCalledExactlyOnceWith(ADA);
  });

  it("goes back to text from an empty first argument, and the cancel button drops the command", async () => {
    const { user, field } = setup({ commandHost: { ban: vi.fn(), people: [ADA] } });

    await user.type(field, "/zbanuj ");
    await user.keyboard("{Backspace}");
    await nextFrame();
    expect(field).toHaveValue("/zbanuj");
    expect(field).toHaveFocus();
    expect(listed()).toEqual(["/zbanuj"]);

    // Chosen again, and left with Escape: the people list that opened by
    // itself does not hold it back.
    await user.keyboard("{Enter}");
    expect(screen.getByRole("combobox", { name: "osoba" })).toHaveFocus();
    await user.keyboard("{Escape}");
    await nextFrame();
    expect(field).toHaveValue("/zbanuj");

    await user.clear(field);
    await user.type(field, "/zbanuj ");
    await user.click(screen.getByRole("button", { name: "Anuluj komendę" }));
    await nextFrame();
    expect(field).toHaveValue("");
    expect(field).toHaveFocus();
  });
});

describe("Composer command sheet", () => {
  it("opens from the plus where the formatting button is not in the row", async () => {
    const { user, container } = setup({
      onAttach: vi.fn(),
      onGif: vi.fn().mockResolvedValue(undefined),
      commandHost: { openPinned: vi.fn(), openStatus: vi.fn() },
    });
    // What the phone's stylesheet does: formatting moves into the sheet.
    screen.getByRole("button", { name: "Formatowanie" }).style.display = "none";
    const click = vi.spyOn(container.querySelector<HTMLInputElement>('input[type="file"]')!, "click");
    const plus = screen.getByRole("button", { name: "Komendy i dodawanie" });

    await user.click(plus);
    const sheet = screen.getByRole("dialog", { name: "Komendy i dodawanie" });
    const rows = (name: string) =>
      within(within(sheet).getByRole("group", { name })).getAllByRole("button").map((b) => b.textContent);
    expect(rows("Wiadomość")).toEqual([
      " Kod /kod",
      " Formatowanie pogrubienie, kursywa, spoiler",
      " Spoiler /spoiler",
    ]);
    expect(rows("Serwer")).toEqual([" Przypięte /przypięte", " Status /status"]);
    // Files and GIFs as tiles; the voice message keeps its button in the row.
    expect(within(sheet).getByRole("button", { name: "Plik" })).toBeInTheDocument();
    expect(within(sheet).getByRole("button", { name: "GIF" })).toBeInTheDocument();

    await user.click(within(sheet).getByRole("button", { name: "Formatowanie pogrubienie, kursywa, spoiler" }));
    expect(screen.queryByRole("dialog", { name: "Komendy i dodawanie" })).toBeNull();
    expect(screen.getByRole("toolbar", { name: "Formatowanie" })).toBeInTheDocument();

    await user.click(plus);
    await user.click(within(screen.getByRole("dialog", { name: "Komendy i dodawanie" })).getByRole("button", { name: "Plik" }));
    expect(click).toHaveBeenCalledTimes(1);
  });
});
