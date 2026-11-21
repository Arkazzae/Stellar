import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { ServerCreateModal } from "@/components/settings/server/ServerCreateModal";
import { LanguageProvider } from "@/state/LanguageProvider";
import { useLanguage } from "@/state/hooks";

beforeEach(() => {
  localStorage.clear();
});

/** Names the server, which the channels step needs, and moves on to it. */
async function toChannels(user: ReturnType<typeof userEvent.setup>, field: RegExp, next: string) {
  await user.type(screen.getByRole("textbox", { name: field }), "Crew");
  await user.click(screen.getByRole("button", { name: next }));
}

describe("the create-server dialog's starter channel names", () => {
  it("are offered in English to someone using the English interface", async () => {
    const user = userEvent.setup();
    const onCreate = vi.fn().mockResolvedValue(undefined);

    render(
      <LanguageProvider initialLocale="en">
        <ServerCreateModal open onClose={vi.fn()} onCreate={onCreate} />
      </LanguageProvider>,
    );

    await toChannels(user, /^Server name/, "Next");

    expect(screen.getByRole("textbox", { name: "Text channel" })).toHaveValue("general");
    expect(screen.getByRole("textbox", { name: "Voice channel name" })).toHaveValue("voice");

    await user.click(screen.getByRole("button", { name: "Create now" }));

    expect(onCreate).toHaveBeenCalledWith(
      expect.objectContaining({ textChannelName: "general", voiceChannelName: "voice" }),
    );
  });

  it("follow the language chosen while the dialog was closed", async () => {
    const user = userEvent.setup();

    function Harness() {
      const { update } = useLanguage();
      const [open, setOpen] = useState(true);
      return (
        <>
          <button onClick={() => update({ locale: "en" })}>english</button>
          <button onClick={() => setOpen((current) => !current)}>toggle</button>
          <ServerCreateModal open={open} onClose={vi.fn()} onCreate={vi.fn()} />
        </>
      );
    }

    render(
      <LanguageProvider initialLocale="pl">
        <Harness />
      </LanguageProvider>,
    );
    await toChannels(user, /^Nazwa serwera/, "Dalej");
    expect(screen.getByRole("textbox", { name: "Kanał tekstowy" })).toHaveValue("ogólny");

    await user.click(screen.getByRole("button", { name: "toggle" }));
    await user.click(screen.getByRole("button", { name: "english" }));
    await user.click(screen.getByRole("button", { name: "toggle" }));
    await toChannels(user, /^Server name/, "Next");

    expect(screen.getByRole("textbox", { name: "Text channel" })).toHaveValue("general");
    expect(screen.getByRole("textbox", { name: "Voice channel name" })).toHaveValue("voice");
  });
});
