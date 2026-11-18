import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { polishAsRead } from "@/test/translators";

import { ProviderConfirmStep } from "./ProviderConfirmStep";

const t = polishAsRead;
const WORK = { id: "work", displayName: "Work" };

describe("ProviderConfirmStep", () => {
  it("leaves for the provider picked, and says why when none is left to pick", async () => {
    const user = userEvent.setup();
    const onConfirm = vi.fn();
    const { rerender } = render(
      <ProviderConfirmStep t={t} providers={[WORK]} pending={false} error={null} onConfirm={onConfirm} />,
    );

    await user.click(screen.getByRole("button", { name: t("security.provider.submit", { provider: "Work" }) }));
    expect(onConfirm).toHaveBeenCalledExactlyOnceWith(WORK);
    expect(screen.queryByText(t("security.provider.none"))).toBeNull();

    rerender(<ProviderConfirmStep t={t} providers={[]} pending={false} error={null} onConfirm={onConfirm} />);
    expect(screen.getByText(t("security.provider.none"))).toBeVisible();
    expect(screen.queryByRole("button", { name: t("security.provider.submit", { provider: "Work" }) })).toBeNull();
  });
});
