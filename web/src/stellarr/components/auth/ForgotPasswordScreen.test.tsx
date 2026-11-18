import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import { polishAsRead } from "@/test/translators";

import { ForgotPasswordScreen, RESEND_AFTER_SECONDS } from "./ForgotPasswordScreen";

const t = polishAsRead;

afterEach(() => {
  vi.useRealTimers();
});

describe("ForgotPasswordScreen", () => {
  it("sends the trimmed address, says where it went, and sends it again only after a minute", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    const onSubmit = vi.fn(async () => undefined);
    render(<ForgotPasswordScreen t={t} instanceName="Stellarr Dev" onSubmit={onSubmit} />);

    await user.type(screen.getByRole("textbox", { name: "E-mail" }), " maja@example.com ");
    await user.click(screen.getByRole("button", { name: t("auth.forgot.submit") }));

    expect(onSubmit).toHaveBeenCalledExactlyOnceWith("maja@example.com");
    expect(
      await screen.findByText(t("auth.forgot.sentNamed", { address: "maja@example.com", instance: "Stellarr Dev" })),
    ).toBeVisible();
    expect(
      screen.getByRole("button", { name: t("auth.forgot.resendIn", { seconds: RESEND_AFTER_SECONDS }) }),
    ).toBeDisabled();

    await act(async () => {
      vi.advanceTimersByTime(RESEND_AFTER_SECONDS * 1000);
    });
    await user.click(screen.getByRole("button", { name: t("auth.forgot.resend") }));

    expect(onSubmit).toHaveBeenCalledTimes(2);
    expect(onSubmit).toHaveBeenLastCalledWith("maja@example.com");
  });

  it("says the request failed and keeps the form so it can be retried", async () => {
    const user = userEvent.setup();
    const onSubmit = vi.fn(async () => {
      throw new Error("offline");
    });
    render(<ForgotPasswordScreen t={t} onSubmit={onSubmit} />);

    await user.type(screen.getByRole("textbox", { name: "E-mail" }), "maja@example.com");
    await user.click(screen.getByRole("button", { name: t("auth.forgot.submit") }));

    expect(await screen.findByRole("alert")).toHaveTextContent(t("auth.forgot.failed"));
    expect(screen.getByRole("textbox", { name: "E-mail" })).toBeEnabled();
  });
});
