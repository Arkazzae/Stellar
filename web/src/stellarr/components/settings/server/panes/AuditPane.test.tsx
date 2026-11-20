import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { AuditEntry } from "@/ports";

import { AUDIT_PAGE, AuditPane } from "./AuditPane";

/** A local-time moment, so the day a row falls on does not depend on the zone. */
function local(month: number, day: number, hour: number, minute = 0): string {
  return new Date(2026, month - 1, day, hour, minute).toISOString();
}

const MAJA = { id: "u-maja", username: "maja", displayName: "Maja", presence: "online" as const };
const JAN = { id: "u-jan", username: "jan", displayName: "Jan", presence: "online" as const };

const ENTRIES: AuditEntry[] = [
  {
    id: "a",
    summary: "Maja updated the role Ekipa",
    at: local(8, 6, 18, 40),
    actor: MAJA,
    action: "role.update",
    target: { type: "role", id: "r-1", name: "Ekipa" },
    changes: [
      { field: "color", from: "#8b7cf6", to: "#f76b15", fromColor: "#8b7cf6", toColor: "#f76b15" },
      { field: "permissions", added: ["manageBans"], removed: ["manageChannels"] },
    ],
  },
  {
    id: "b",
    summary: "Jan banned Ola",
    at: local(8, 5, 22, 15),
    actor: JAN,
    action: "member.ban",
    target: { type: "member", id: "u-ola", name: "Ola" },
    changes: [{ field: "reason", to: "Spam" }],
  },
  { id: "c", summary: "Created invite link for #ogólny", at: local(8, 1, 9), actor: MAJA },
];

describe("AuditPane", () => {
  beforeEach(() => {
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.setSystemTime(new Date(2026, 7, 6, 20, 0));
  });

  afterEach(() => vi.useRealTimers());

  it("puts the entries in a card per day: today, yesterday, then the date", () => {
    render(<AuditPane supported entries={ENTRIES} />);

    expect(screen.getByRole("region", { name: "Dziś" })).toBeVisible();
    expect(screen.getByRole("region", { name: "Wczoraj" })).toBeVisible();
    expect(screen.getByRole("region", { name: "1.08.2026" })).toBeVisible();
  });

  it("says each line in the reader's language, with who and to whom in bold", () => {
    render(<AuditPane supported entries={ENTRIES} />);

    const line = screen.getByText((_, node) => node?.textContent === "Maja zaktualizował(a) rolę Ekipa" && node.tagName === "B");
    expect(within(line).getByText("Maja", { selector: "strong" })).toBeVisible();
    expect(within(line).getByText("Ekipa", { selector: "strong" })).toBeVisible();
    expect(screen.getByText("18:40 · Kolor, Uprawnienia")).toBeVisible();
  });

  it("keeps an older line as the sentence it was written as, with its author", () => {
    render(<AuditPane supported entries={ENTRIES} />);

    expect(screen.getByText("Created invite link for #ogólny")).toBeVisible();
    expect(screen.getByText("09:00 · Maja")).toBeVisible();
  });

  it("opens a line to what changed", async () => {
    vi.useRealTimers();
    const user = userEvent.setup();
    render(<AuditPane supported entries={ENTRIES} />);
    const row = screen.getByRole("button", { name: /Maja zaktualizował\(a\) rolę Ekipa/ });
    const detail = document.getElementById(row.getAttribute("aria-controls")!)!;
    expect(detail).not.toBeVisible();

    await user.click(row);

    expect(row).toHaveAttribute("aria-expanded", "true");
    expect(within(detail).getByText("+ Zarządzanie banami")).toBeVisible();
    expect(within(detail).getByText("− Zarządzanie kanałami")).toBeVisible();
    expect(within(detail).getByText("#f76b15")).toBeVisible();
  });

  it("filters by who did it and by what was done", async () => {
    vi.useRealTimers();
    const user = userEvent.setup();
    render(<AuditPane supported entries={ENTRIES} />);

    await user.click(screen.getByRole("combobox", { name: "Filtruj po akcji" }));
    await user.click(screen.getByRole("option", { name: "Moderacja" }));
    expect(screen.queryByText(/rolę Ekipa/)).toBeNull();
    expect(screen.getByText(/zbanował\(a\)/)).toBeVisible();

    await user.click(screen.getByRole("combobox", { name: "Filtruj po osobie" }));
    await user.click(screen.getByRole("option", { name: "Maja" }));
    expect(screen.getByText("Nic nie pasuje do tych filtrów.")).toBeVisible();

    await user.click(screen.getByRole("button", { name: "Wyczyść filtry" }));
    expect(screen.getAllByRole("region")).toHaveLength(3);
  });

  it("loads older entries from the oldest one shown, until the log ends", async () => {
    vi.useRealTimers();
    const user = userEvent.setup();
    const page = Array.from({ length: AUDIT_PAGE }, (_, index): AuditEntry => ({
      id: `p${index}`,
      summary: `Wpis ${index}`,
      at: new Date(2026, 6, 30, 12, 59 - index).toISOString(),
    }));
    const onLoadOlder = vi.fn().mockResolvedValue([{ id: "old", summary: "Najstarszy", at: new Date(2026, 5, 1).toISOString() }]);
    render(<AuditPane supported entries={page} onLoadOlder={onLoadOlder} />);

    await user.click(screen.getByRole("button", { name: "Wczytaj starsze" }));

    expect(onLoadOlder).toHaveBeenCalledWith(page[AUDIT_PAGE - 1]!.at);
    await waitFor(() => expect(screen.getByText("Najstarszy")).toBeVisible());
    expect(screen.getByText("To cały dziennik.")).toBeVisible();
  });

  it("asks for the newest page whenever it is opened, and says when nothing was recorded", () => {
    const onShow = vi.fn();
    render(<AuditPane supported entries={[]} onShow={onShow} />);

    expect(onShow).toHaveBeenCalledOnce();
    expect(screen.getByText("Nic jeszcze nie zapisano.")).toBeVisible();
  });
});
