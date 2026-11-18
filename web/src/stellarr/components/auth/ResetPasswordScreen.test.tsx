import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { polishAsRead } from "@/test/translators";

import { ResetPasswordScreen } from "./ResetPasswordScreen";

const t = polishAsRead;
const newPassword = () => screen.getByLabelText(t("auth.reset.newPassword"));
const submit = () => screen.getByRole("button", { name: t("auth.reset.submit") });

describe("ResetPasswordScreen", () => {
  it("refuses a password under 8 characters before calling onSubmit", async () => {
    const user = userEvent.setup();
    const onSubmit = vi.fn(async () => undefined);
    render(<ResetPasswordScreen token="abc" t={t} onSubmit={onSubmit} />);

    await user.type(newPassword(), "short");
    await user.click(submit());

    expect(onSubmit).not.toHaveBeenCalled();
    expect(screen.getByRole("alert")).toHaveTextContent(t("auth.password.length"));
  });

  it("sends the token with the new password, then says other sessions ended and leads to the login", async () => {
    const user = userEvent.setup();
    const onSubmit = vi.fn(async () => undefined);
    render(<ResetPasswordScreen token="abc" t={t} onSubmit={onSubmit} />);

    await user.type(newPassword(), "haslo1234");
    await user.click(submit());

    expect(onSubmit).toHaveBeenCalledExactlyOnceWith("abc", "haslo1234");
    expect(await screen.findByText(t("auth.reset.success"))).toBeVisible();
    expect(screen.getByRole("link", { name: t("auth.reset.goLogin") })).toHaveAttribute("href", "/login");
  });

  it("explains that the link is no longer valid when the request is rejected", async () => {
    const user = userEvent.setup();
    const onSubmit = vi.fn(async () => {
      throw new Error("expired");
    });
    render(<ResetPasswordScreen token="abc" t={t} onSubmit={onSubmit} />);

    await user.type(newPassword(), "haslo1234");
    await user.click(submit());

    expect(await screen.findByRole("alert")).toHaveTextContent(t("auth.reset.invalid"));
    expect(newPassword()).toBeEnabled();
  });
});
