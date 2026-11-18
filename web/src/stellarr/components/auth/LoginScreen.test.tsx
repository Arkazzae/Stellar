import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { polishAsRead } from "@/test/translators";

import { LoginScreen } from "./LoginScreen";

const t = polishAsRead;
const identifier = () => screen.getByRole("textbox", { name: "Login lub e-mail" });
const password = () => screen.getByLabelText("Hasło");
const submit = () => screen.getByRole("button", { name: "Zaloguj się" });

describe("LoginScreen", () => {
  it("submits the entered credentials with the login trimmed", async () => {
    const user = userEvent.setup();
    const onSubmit = vi.fn();
    render(<LoginScreen onSubmit={onSubmit} />);

    await user.type(identifier(), "  maja  ");
    await user.type(password(), "haslo123");
    await user.click(submit());

    expect(onSubmit).toHaveBeenCalledExactlyOnceWith("maja", "haslo123");
  });

  it("offers to keep the session only where the page holds that choice", async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    const { rerender } = render(<LoginScreen onSubmit={vi.fn()} />);
    // The desktop app keeps its session; it passes no choice.
    expect(screen.queryByRole("checkbox")).toBeNull();

    rerender(<LoginScreen onSubmit={vi.fn()} keepSignedIn={{ checked: true, onChange }} />);
    await user.click(screen.getByRole("checkbox", { name: t("auth.login.keepSignedIn") }));

    expect(onChange).toHaveBeenCalledExactlyOnceWith(false);
  });

  it("asks for both fields before submitting anything", async () => {
    const user = userEvent.setup();
    const onSubmit = vi.fn();
    render(<LoginScreen onSubmit={onSubmit} />);

    await user.type(identifier(), "maja");
    await user.click(submit());

    expect(onSubmit).not.toHaveBeenCalled();
    expect(screen.getByRole("alert")).toHaveTextContent(t("auth.login.missing"));
    expect(identifier()).toHaveFocus();
    expect(identifier()).toHaveAttribute("aria-invalid", "true");
  });

  it("locks the form and ignores a submit while a request is pending", async () => {
    const user = userEvent.setup();
    const onSubmit = vi.fn();
    render(<LoginScreen onSubmit={onSubmit} pending />);

    expect(identifier()).toBeDisabled();
    expect(password()).toBeDisabled();
    await user.click(screen.getByRole("button", { name: "Logowanie…" }));
    expect(onSubmit).not.toHaveBeenCalled();
  });

  it("runs the desktop app's external sign-in handler instead of following a link", async () => {
    const user = userEvent.setup();
    const onClick = vi.fn();
    const { rerender } = render(
      <LoginScreen onSubmit={vi.fn()} externalLogin={{ label: "Zaloguj przez przeglądarkę", onClick }} />,
    );

    await user.click(screen.getByRole("button", { name: "Zaloguj przez przeglądarkę" }));
    expect(onClick).toHaveBeenCalledOnce();

    rerender(
      <LoginScreen
        onSubmit={vi.fn()}
        externalLogin={{ label: "Zaloguj przez przeglądarkę", onClick, pending: true }}
      />,
    );
    expect(screen.getByRole("button", { name: "Zaloguj przez przeglądarkę" })).toBeDisabled();
  });

  it("shows the password on request, and with a native broker lifts the mask instead of the type", async () => {
    const user = userEvent.setup();
    const { unmount } = render(<LoginScreen onSubmit={vi.fn()} passwordAutoComplete="new-password" />);
    expect(password()).toHaveAttribute("type", "password");
    expect(password()).toHaveAttribute("autocomplete", "new-password");
    await user.click(screen.getByRole("button", { name: t("auth.password.show") }));
    expect(password()).toHaveAttribute("type", "text");
    unmount();

    // The desktop app's broker owns the credentials: the browser must not
    // treat the field as a password at any point.
    render(<LoginScreen onSubmit={vi.fn()} nativeCredentialBroker />);
    const field = password();
    expect(field).toHaveAttribute("type", "text");
    expect(field).toHaveAttribute("autocomplete", "off");
    expect(field).toHaveAttribute("data-1p-ignore", "true");
    const masked = field.className;
    await user.click(screen.getByRole("button", { name: t("auth.password.show") }));
    expect(field).toHaveAttribute("type", "text");
    expect(field.className).not.toBe(masked);
  });

  it("refuses an invite field that holds no invite, and opens one that does", async () => {
    const user = userEvent.setup();
    const onInviteCode = vi.fn();
    render(<LoginScreen onSubmit={vi.fn()} onInviteCode={onInviteCode} />);

    await user.click(screen.getByRole("button", { name: t("auth.login.haveInvite") }));
    const field = screen.getByRole("textbox", { name: t("auth.invite.linkLabel") });
    await user.type(field, "https://chat.example/login");
    await user.click(screen.getByRole("button", { name: t("auth.invite.open") }));
    expect(onInviteCode).not.toHaveBeenCalled();
    expect(screen.getByRole("alert")).toHaveTextContent(t("auth.invite.notALink"));

    await user.clear(field);
    await user.type(field, " k7qm2xwp ");
    await user.click(screen.getByRole("button", { name: t("auth.invite.open") }));
    expect(onInviteCode).toHaveBeenCalledExactlyOnceWith("k7qm2xwp");
  });
});
