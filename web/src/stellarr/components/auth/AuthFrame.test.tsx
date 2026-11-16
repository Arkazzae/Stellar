import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { describe, expect, it } from "vitest";

import { AuthFrame } from "./AuthFrame";

/** A form that turns into its confirmation, as the reset and recovery screens do. */
function SendsOnce() {
  const [sent, setSent] = useState(false);
  return (
    <AuthFrame title="Odzyskaj dostęp" description="Podaj e-mail.">
      {sent ? (
        <p role="status">Wysłano.</p>
      ) : (
        <button type="button" onClick={() => setSent(true)}>
          Wyślij
        </button>
      )}
    </AuthFrame>
  );
}

function ServerStep({ onNext }: { onNext: () => void }) {
  return (
    <AuthFrame title="Utwórz serwer" description="Krok 1.">
      <button type="button" onClick={onNext}>
        Dalej
      </button>
    </AuthFrame>
  );
}

function AccountStep({ onBack }: { onBack: () => void }) {
  return (
    <AuthFrame title="Załóż konto" description="Krok 2.">
      <button type="button" onClick={onBack}>
        Wróć
      </button>
    </AuthFrame>
  );
}

/** Two steps, each a component with a frame of its own, as the first registration has. */
function TwoSteps() {
  const [step, setStep] = useState(1);
  return step === 1 ? <ServerStep onNext={() => setStep(2)} /> : <AccountStep onBack={() => setStep(1)} />;
}

describe("AuthFrame", () => {
  it("takes no focus when the page opens", () => {
    render(<SendsOnce />);

    expect(document.body).toHaveFocus();
  });

  it("gives its heading the focus the content took with it", async () => {
    const user = userEvent.setup();
    render(<SendsOnce />);

    await user.click(screen.getByRole("button", { name: "Wyślij" }));

    expect(screen.getByRole("heading", { name: "Odzyskaj dostęp" })).toHaveFocus();
  });

  it("takes the focus a frame it replaced held, in both directions", async () => {
    const user = userEvent.setup();
    render(<TwoSteps />);

    await user.click(screen.getByRole("button", { name: "Dalej" }));
    expect(screen.getByRole("heading", { name: "Załóż konto" })).toHaveFocus();

    await user.click(screen.getByRole("button", { name: "Wróć" }));
    expect(screen.getByRole("heading", { name: "Utwórz serwer" })).toHaveFocus();
  });

  it("leaves the focus where it is when the focused element stays", async () => {
    const user = userEvent.setup();
    function Counter() {
      const [count, setCount] = useState(0);
      return (
        <AuthFrame title={`Krok ${count}`} description="">
          <button type="button" onClick={() => setCount(count + 1)}>
            Dalej
          </button>
        </AuthFrame>
      );
    }
    render(<Counter />);

    await user.click(screen.getByRole("button", { name: "Dalej" }));

    expect(screen.getByRole("button", { name: "Dalej" })).toHaveFocus();
  });
});
