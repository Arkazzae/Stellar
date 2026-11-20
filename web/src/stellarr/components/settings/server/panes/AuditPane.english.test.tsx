import { render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { translateMessage } from "@/i18n";

import { AuditPane } from "./AuditPane";

vi.mock("@/state/hooks", () => ({
  useLanguage: () => ({
    locale: "en",
    t: (key: Parameters<typeof translateMessage>[1], values?: Record<string, string | number>) =>
      translateMessage("en", key, values),
  }),
}));

describe("AuditPane in English", () => {
  beforeEach(() => {
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.setSystemTime(new Date(2026, 7, 6, 20, 0));
  });

  afterEach(() => vi.useRealTimers());

  it("names the days and says each line in English, whatever the server's language", () => {
    const maja = { id: "u-maja", username: "maja", displayName: "Maja", presence: "online" as const };
    render(
      <AuditPane
        supported
        entries={[
          {
            id: "a",
            summary: "Maja utworzył(a) kanał #ogolny",
            at: new Date(2026, 7, 6, 18, 40).toISOString(),
            actor: maja,
            action: "channel.create",
            target: { type: "channel", name: "ogolny" },
          },
          { id: "b", summary: "Jan zbanował(a) Olę", at: new Date(2026, 7, 5, 22, 15).toISOString() },
          { id: "c", summary: "Jan renamed the server", at: new Date(2026, 7, 1, 9).toISOString() },
        ]}
      />,
    );

    expect(screen.getByRole("region", { name: "Today" })).toBeVisible();
    expect(screen.getByRole("region", { name: "Yesterday" })).toBeVisible();
    expect(screen.getByRole("region", { name: "1/08/2026" })).toBeVisible();
    expect(screen.getByText((_, node) => node?.tagName === "B" && node.textContent === "Maja created the channel #ogolny")).toBeVisible();
    expect(screen.getByText("Jan zbanował(a) Olę")).toBeVisible();
  });
});
