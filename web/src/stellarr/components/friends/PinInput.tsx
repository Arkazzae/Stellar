import { useEffect, useId, useRef, useState, type RefObject } from "react";

const LENGTH = 6;

/**
 * Focuses a dialog's field once the dialog has settled. The dialog shell puts
 * the focus on its first control (the close button) when it opens and again
 * when it has faded in, so a field's own autofocus would be taken away; this
 * waits for both before taking the focus to where typing goes.
 */
export function useSettledFocus(ref: RefObject<HTMLElement | null>): void {
  useEffect(() => {
    let second = 0;
    const first = requestAnimationFrame(() => {
      second = requestAnimationFrame(() => ref.current?.focus());
    });
    return () => {
      cancelAnimationFrame(first);
      cancelAnimationFrame(second);
    };
  }, [ref]);
}

export interface PinInputProps {
  value: string;
  onChange(value: string): void;
  label: string;
  invalid?: boolean;
  disabled?: boolean;
  /** Id of the sentence that explains an error. */
  describedBy?: string;
}

/**
 * Six digits, drawn as six boxes.
 *
 * One real field lies over the boxes, so typing, pasting, a phone's number pad
 * and a screen reader all meet an ordinary input; the boxes only draw how far
 * the PIN has got. It is a text field rather than a password one, so a
 * password manager does not offer to save the PIN as the account's password.
 */
export function PinInput({ value, onChange, label, invalid = false, disabled = false, describedBy }: PinInputProps) {
  const id = useId();
  const field = useRef<HTMLInputElement>(null);
  const [focused, setFocused] = useState(false);
  useSettledFocus(field);
  const boxes = Array.from({ length: LENGTH }, (_, index) => {
    const filled = index < value.length;
    const current = focused && index === Math.min(value.length, LENGTH - 1) && !filled;
    return <span key={index} className={filled ? "pin-box fill" : current ? "pin-box cur" : "pin-box"} />;
  });

  return (
    <div className={invalid ? "pin-input err" : "pin-input"}>
      <label className="sr-only" htmlFor={id}>
        {label}
      </label>
      <input
        ref={field}
        id={id}
        className="pin-field"
        type="text"
        inputMode="numeric"
        autoComplete="off"
        spellCheck={false}
        maxLength={LENGTH}
        value={value}
        disabled={disabled}
        aria-invalid={invalid || undefined}
        aria-describedby={describedBy}
        onFocus={() => setFocused(true)}
        onBlur={() => setFocused(false)}
        onChange={(event) => onChange(event.target.value.replace(/\D/g, "").slice(0, LENGTH))}
      />
      <span className="pin-boxes" aria-hidden="true">
        {boxes}
      </span>
    </div>
  );
}
