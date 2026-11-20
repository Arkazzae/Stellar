import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { OverviewPane, type OverviewPaneProps } from "./OverviewPane";

function props(overrides: Partial<OverviewPaneProps> = {}): OverviewPaneProps {
  return {
    supported: false,
    iconSupported: true,
    name: "Stellarr",
    mark: "ST",
    createdAt: "2026-03-06T12:00:00Z",
    memberCount: 4,
    channelCount: 3,
    description: "Prywatna przestrzeń.",
    isPublic: false,
    onSave: vi.fn().mockResolvedValue(undefined),
    onPickIcon: vi.fn().mockResolvedValue(undefined),
    onRemoveIcon: vi.fn().mockResolvedValue(undefined),
    ...overrides,
  };
}

describe("OverviewPane", () => {
  beforeEach(() => {
    vi.spyOn(URL, "createObjectURL").mockReturnValue("blob:server-icon");
    vi.spyOn(URL, "revokeObjectURL").mockImplementation(() => {});
  });

  afterEach(() => vi.restoreAllMocks());

  it("opens with the card the server is seen as", () => {
    render(<OverviewPane {...props()} />);

    const card = screen.getByRole("region", { name: "Jak serwer wygląda w Odkrywaniu i w zaproszeniach" });
    expect(within(card).getByText("Stellarr")).toBeVisible();
    expect(within(card).getByText("Prywatna przestrzeń.")).toBeVisible();
    expect(within(card).getByText("4 osoby")).toBeVisible();
    expect(within(card).getByText("3 kanały")).toBeVisible();
    expect(within(card).getByText("Tylko z zaproszeniem")).toBeVisible();
  });

  it("shows what is being typed in the card before it is saved", async () => {
    const user = userEvent.setup();
    render(<OverviewPane {...props({ supported: true })} />);
    const description = screen.getByRole("textbox", { name: /^Opis/ });

    await user.clear(description);
    expect(screen.getByText("Brak opisu")).toBeVisible();
    await user.type(description, "Nowy opis");

    expect(within(screen.getByRole("region", { name: /Jak serwer wygląda/ })).getByText("Nowy opis")).toBeVisible();
    expect(screen.getByText("9/280")).toBeVisible();
  });

  it("uploads a supported image from the card's icon", async () => {
    const user = userEvent.setup();
    const p = props();
    const { container } = render(<OverviewPane {...p} />);
    const input = container.querySelector('input[type="file"]') as HTMLInputElement;
    const file = new File(["png"], "orbit.png", { type: "image/png" });

    await user.upload(input, file);

    await waitFor(() => expect(p.onPickIcon).toHaveBeenCalledWith(file));
    expect(URL.createObjectURL).toHaveBeenCalledWith(file);
    expect(URL.revokeObjectURL).toHaveBeenCalledWith("blob:server-icon");
  });

  it("shows the persisted icon and lets a manager remove it", async () => {
    const user = userEvent.setup();
    const p = props({ iconUrl: "/assets/server-icon" });
    const { container } = render(<OverviewPane {...p} />);

    expect(container.querySelector(".ss-card .ss-mark img")).toHaveAttribute("src", "/assets/server-icon");
    await user.click(screen.getByRole("button", { name: "Usuń" }));

    expect(p.onRemoveIcon).toHaveBeenCalledOnce();
  });

  it("rejects an unsupported file before calling the backend", () => {
    const p = props();
    const { container } = render(<OverviewPane {...p} />);
    const input = container.querySelector('input[type="file"]') as HTMLInputElement;

    fireEvent.change(input, {
      target: { files: [new File(["text"], "notes.txt", { type: "text/plain" })] },
    });

    expect(screen.getByRole("alert")).toHaveTextContent("Ikona musi być plikiem JPEG, PNG, GIF lub WebP.");
    expect(p.onPickIcon).not.toHaveBeenCalled();
  });

  it("keeps icon actions disabled without manage-server support", () => {
    render(<OverviewPane {...props({ iconSupported: false })} />);

    expect(screen.getByRole("button", { name: "Dodaj ikonę serwera" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Wgraj obraz" })).toBeDisabled();
  });

  it("saves the name and description through the save bar", async () => {
    const user = userEvent.setup();
    const onSave = vi.fn().mockResolvedValue(undefined);
    render(<OverviewPane {...props({ supported: true, onSave })} />);
    const name = screen.getByRole("textbox", { name: /^Nazwa/ });

    await user.clear(name);
    await user.type(name, "Nowa Orbita");
    await user.click(screen.getByRole("button", { name: "Zapisz zmiany" }));

    await waitFor(() => expect(onSave).toHaveBeenCalledWith({ name: "Nowa Orbita", description: "Prywatna przestrzeń." }));
    expect(screen.getByRole("region", { name: "Zmiany" })).toHaveTextContent("Zapisano");
  });

  it("saves visibility at once, as every switch does", async () => {
    const user = userEvent.setup();
    const onSave = vi.fn().mockResolvedValue(undefined);
    render(<OverviewPane {...props({ supported: true, onSave })} />);

    await user.click(screen.getByRole("switch", { name: "Serwer publiczny" }));

    expect(onSave).toHaveBeenCalledWith({ isPublic: true });
    expect(screen.queryByRole("region", { name: "Zmiany" })).toBeNull();
  });

  it("names the owner, and offers the owner a transfer to a member they pick", async () => {
    const user = userEvent.setup();
    const onTransfer = vi.fn().mockResolvedValue(undefined);
    render(
      <OverviewPane
        {...props({
          supported: true,
          owner: { id: "u-1", displayName: "Maja" },
          viewerIsOwner: true,
          transfer: { candidates: [{ id: "u-2", name: "Jan", role: "Ekipa" }], onTransfer },
        })}
      />,
    );

    expect(screen.getByText("Jesteś właścicielem serwera")).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Przekaż serwer" }));
    const card = screen.getByRole("dialog", { name: "Przekazać serwer?" });
    expect(within(card).getByRole("button", { name: "Przekaż" })).toBeDisabled();

    await user.click(within(card).getByRole("combobox", { name: "NOWY WŁAŚCICIEL" }));
    await user.click(screen.getByRole("option", { name: /Jan/ }));
    await user.click(within(card).getByRole("button", { name: "Przekaż" }));

    await waitFor(() => expect(onTransfer).toHaveBeenCalledWith("u-2"));
  });

  it("only names the owner to someone who is not", () => {
    render(<OverviewPane {...props({ owner: { id: "u-1", displayName: "Maja" } })} />);

    expect(screen.getByText("Maja jest właścicielem serwera")).toBeVisible();
    expect(screen.queryByRole("button", { name: "Przekaż serwer" })).toBeNull();
  });
});
