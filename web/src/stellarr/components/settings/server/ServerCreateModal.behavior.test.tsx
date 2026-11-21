import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { ServerCreateModal } from "./ServerCreateModal";

const nameField = () => screen.getByRole("textbox", { name: /^Nazwa serwera/ });
const button = (name: string) => screen.getByRole("button", { name });
const never = () => new Promise<void>(() => {});

const DEFAULTS = {
  description: "",
  isPublic: false,
  textChannelName: "ogólny",
  voiceChannelName: "rozmowa",
  mentionsOnly: true,
  voiceJoinSound: true,
};

describe("ServerCreateModal", () => {
  it("creates the server at once from the first step, with the defaults for the rest", async () => {
    const user = userEvent.setup();
    const onCreate = vi.fn(never);

    render(<ServerCreateModal open onClose={vi.fn()} onCreate={onCreate} />);

    await user.type(nameField(), "  Załoga  ");
    await user.click(button("Utwórz od razu"));

    expect(onCreate).toHaveBeenCalledExactlyOnceWith({ name: "Załoga", ...DEFAULTS });
  });

  it("neither moves on nor creates without a name", async () => {
    const user = userEvent.setup();
    const onCreate = vi.fn(never);

    render(<ServerCreateModal open onClose={vi.fn()} onCreate={onCreate} />);

    await user.type(nameField(), "   ");
    expect(button("Dalej")).toBeDisabled();
    expect(button("Utwórz od razu")).toBeDisabled();

    await user.keyboard("{Enter}");
    expect(screen.getByRole("heading", { name: "Nazwij serwer" })).toBeInTheDocument();
    expect(onCreate).not.toHaveBeenCalled();
  });

  it("takes the draft through the three steps, back and forth, and creates what was chosen", async () => {
    const user = userEvent.setup();
    const onCreate = vi.fn(never);
    const onClose = vi.fn();

    render(<ServerCreateModal open onClose={onClose} onCreate={onCreate} />);

    await user.type(nameField(), "Załoga");
    await user.type(screen.getByRole("textbox", { name: /^Opis/ }), "  Wieczorne granie  ");
    await user.click(button("Dalej"));

    const text = screen.getByRole("textbox", { name: "Kanał tekstowy" });
    await user.clear(text);
    await user.type(text, "Ogłoszenia Ekipy");
    await user.click(screen.getByRole("switch", { name: "Kanał głosowy" }));
    expect(screen.queryByRole("textbox", { name: "Nazwa kanału głosowego" })).toBeNull();

    // Going back keeps what was typed on every step.
    await user.click(button("Wstecz"));
    expect(nameField()).toHaveValue("Załoga");
    await user.click(button("Dalej"));
    expect(screen.getByRole("textbox", { name: "Kanał tekstowy" })).toHaveValue("Ogłoszenia Ekipy");
    await user.click(button("Dalej"));

    await user.click(screen.getByRole("radio", { name: "Publiczny" }));
    await user.click(screen.getByRole("radio", { name: "Wszystko" }));
    await user.click(screen.getByRole("switch", { name: "Dźwięk dołączenia" }));
    await user.click(button("Utwórz serwer"));

    expect(onCreate).toHaveBeenCalledExactlyOnceWith({
      name: "Załoga",
      description: "Wieczorne granie",
      isPublic: true,
      textChannelName: "ogłoszenia-ekipy",
      voiceChannelName: "",
      mentionsOnly: false,
      voiceJoinSound: false,
    });

    // While the server is being made the dialog cannot be put away.
    expect(button("Tworzenie serwera…")).toBeDisabled();
    await user.keyboard("{Escape}");
    expect(onClose).not.toHaveBeenCalled();
  });

  it("does not move on while the two channels share a name", async () => {
    const user = userEvent.setup();
    const onCreate = vi.fn(never);

    render(<ServerCreateModal open onClose={vi.fn()} onCreate={onCreate} />);

    await user.type(nameField(), "Załoga");
    await user.click(button("Dalej"));
    const text = screen.getByRole("textbox", { name: "Kanał tekstowy" });
    await user.clear(text);
    await user.type(text, "Rozmowa");

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Kanał tekstowy i głosowy nie mogą mieć tej samej nazwy.",
    );
    expect(screen.getByRole("textbox", { name: "Nazwa kanału głosowego" })).toHaveAttribute(
      "aria-invalid",
      "true",
    );
    expect(button("Dalej")).toBeDisabled();
    expect(button("Utwórz od razu")).toBeDisabled();

    // Creating at once from the first step leads back to the clash instead.
    await user.click(button("Wstecz"));
    await user.click(button("Utwórz od razu"));
    expect(screen.getByRole("heading", { name: "Kanały na start" })).toBeInTheDocument();
    expect(onCreate).not.toHaveBeenCalled();
  });

  it("returns to the first step with the refusal under the name", async () => {
    const user = userEvent.setup();
    const onCreate = vi.fn().mockRejectedValue(new Error("Ta nazwa jest już zajęta."));

    render(<ServerCreateModal open onClose={vi.fn()} onCreate={onCreate} />);

    await user.type(nameField(), "Załoga");
    await user.click(button("Dalej"));
    await user.click(button("Dalej"));
    await user.click(button("Utwórz serwer"));

    expect(await screen.findByRole("alert")).toHaveTextContent("Ta nazwa jest już zajęta.");
    expect(screen.getByRole("heading", { name: "Nazwij serwer" })).toBeInTheDocument();
    expect(nameField()).toHaveValue("Załoga");
    expect(nameField()).toHaveAttribute("aria-invalid", "true");
    expect(nameField()).toHaveFocus();
  });

  it("words the refusal itself when it carries no message", async () => {
    const user = userEvent.setup();
    const onCreate = vi.fn().mockRejectedValue("refused");

    render(<ServerCreateModal open onClose={vi.fn()} onCreate={onCreate} />);

    await user.type(nameField(), "Załoga");
    await user.click(button("Utwórz od razu"));

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Nie udało się utworzyć serwera. Sprawdź dane i spróbuj ponownie.",
    );
  });

  it("hands out the link to the server it made and closes on Done", async () => {
    const user = userEvent.setup();
    const onCopyInvite = vi.fn().mockResolvedValue(undefined);
    const onClose = vi.fn();
    const url = "https://chat.example/invite/K7fQ2m";

    render(
      <ServerCreateModal
        open
        onClose={onClose}
        onCreate={vi.fn()}
        created={{ name: "Załoga" }}
        invite={{ state: "ready", url }}
        onCopyInvite={onCopyInvite}
      />,
    );

    expect(screen.getByRole("status", { name: "Link zaproszenia" })).toHaveTextContent(url);
    await user.click(button("Kopiuj"));
    expect(onCopyInvite).toHaveBeenCalledExactlyOnceWith(url);
    expect(button("Skopiowano")).toBeInTheDocument();

    await user.click(button("Gotowe"));
    expect(onClose).toHaveBeenCalledOnce();
  });

  it("starts from a blank draft on the first step every time it opens", async () => {
    const user = userEvent.setup();
    const props = { onClose: vi.fn(), onCreate: vi.fn() };
    const { rerender } = render(<ServerCreateModal open {...props} />);

    await user.type(nameField(), "Załoga");
    await user.click(button("Dalej"));
    await user.click(button("Dalej"));
    await user.click(screen.getByRole("radio", { name: "Publiczny" }));

    rerender(<ServerCreateModal open={false} {...props} />);
    rerender(<ServerCreateModal open {...props} />);

    expect(nameField()).toHaveValue("");
    await user.type(nameField(), "Załoga");
    await user.click(button("Dalej"));
    await user.click(button("Dalej"));
    expect(screen.getByRole("radio", { name: "Publiczny" })).toHaveAttribute("aria-checked", "false");
  });
});
