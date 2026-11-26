import { fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";

import { ActivityDevPreview } from "./ActivityDevPreview";

vi.mock("@/state/LanguageProvider", () => ({
  useLanguage: () => ({ t: (key: string) => key }),
}));

afterEach(() => vi.unstubAllGlobals());

it("opens the poll in a frame that can submit votes and keeps the invite in the parent", async () => {
  const src = "https://draft.activity.example/_stellarr/launch?ticket=once";
  const fetch = vi.fn().mockResolvedValue(Response.json({ title: "Shared Poll", src, access: "lease" }));
  vi.stubGlobal("fetch", fetch);
  render(<ActivityDevPreview channel="voice-room" />);
  fireEvent.click(screen.getByRole("button", { name: "stage.devPreview" }));
  fireEvent.change(screen.getByLabelText("stage.previewCode"), { target: { value: "private-code" } });
  fireEvent.click(screen.getByRole("button", { name: "stage.openPreview" }));
  const frame = await screen.findByTitle("Shared Poll");
  expect(fetch).toHaveBeenCalledWith("/_stellarr/dev/launch", expect.objectContaining({
    body: JSON.stringify({ channel: "voice-room", code: "private-code" }),
  }));
  expect(frame).toHaveAttribute("src", src);
  expect(frame.getAttribute("sandbox")?.split(" ")).toContain("allow-forms");
  fireEvent.click(screen.getByRole("button", { name: "stage.closePreview" }));
  expect(screen.queryByTitle("Shared Poll")).toBeNull();
});
