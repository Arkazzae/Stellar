import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { ServerDiscoverModal } from "./ServerDirectoryModals";

describe("ServerDiscoverModal", () => {
  it("filters public servers and joins the selected one", async () => {
    const user = userEvent.setup();
    const onJoinServer = vi.fn().mockResolvedValue(undefined);
    const onClose = vi.fn();

    render(
      <ServerDiscoverModal
        open
        onClose={onClose}
        loadServers={vi.fn().mockResolvedValue([
          { id: "server-a", name: "Alpha", description: "Rozmowy" },
          { id: "server-b", name: "Beta", description: "Wieczorne granie" },
        ])}
        onJoinServer={onJoinServer}
        onJoinInvite={vi.fn()}
      />,
    );

    await screen.findByText("Alpha");
    await user.type(screen.getByRole("searchbox", { name: "Szukaj serwera" }), "beta");

    expect(screen.queryByText("Alpha")).not.toBeInTheDocument();
    expect(screen.getByText("Beta")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Dołącz" }));

    expect(onJoinServer).toHaveBeenCalledWith("server-b");
    expect(onClose).toHaveBeenCalledOnce();
  });

  it("passes a trimmed invite and closes only after joining", async () => {
    const user = userEvent.setup();
    let finish!: () => void;
    const onJoinInvite = vi.fn(
      () => new Promise<void>((resolve) => {
        finish = resolve;
      }),
    );
    const onClose = vi.fn();

    render(
      <ServerDiscoverModal
        open
        onClose={onClose}
        loadServers={vi.fn().mockResolvedValue([])}
        onJoinServer={vi.fn()}
        onJoinInvite={onJoinInvite}
      />,
    );

    await screen.findByText(/Gdy ktoś udostępni serwer publicznie/);
    // The invite field and the public directory share one short card now.
    await user.type(
      screen.getByRole("textbox", { name: /Kod lub link zaproszenia/ }),
      "  https://stellarr.example/invite/abc123  ",
    );
    await user.click(screen.getByRole("button", { name: "Dołącz z zaproszenia" }));

    expect(onJoinInvite).toHaveBeenCalledWith("https://stellarr.example/invite/abc123");
    expect(onClose).not.toHaveBeenCalled();
    expect(screen.getByRole("button", { name: "Dołączanie…" })).toBeDisabled();

    finish();
    await vi.waitFor(() => expect(onClose).toHaveBeenCalledOnce());
  });

  it("offers a retry when loading public servers fails", async () => {
    const user = userEvent.setup();
    const loadServers = vi
      .fn()
      .mockRejectedValueOnce(new Error("Nie udało się pobrać listy."))
      .mockResolvedValueOnce([{ id: "server-a", name: "Alpha" }]);

    render(
      <ServerDiscoverModal
        open
        onClose={vi.fn()}
        loadServers={loadServers}
        onJoinServer={vi.fn()}
        onJoinInvite={vi.fn()}
      />,
    );

    expect(await screen.findByRole("alert")).toHaveTextContent("Nie udało się pobrać listy.");
    await user.click(screen.getByRole("button", { name: "Spróbuj ponownie" }));

    expect(await screen.findByText("Alpha")).toBeInTheDocument();
    expect(loadServers).toHaveBeenCalledTimes(2);
  });
});
