import { useId, type Ref } from "react";

import styles from "./AuthControls.module.css";

export const CODE_LENGTH = 6;

export interface CodeCellsProps {
  label: string;
  value: string;
  onChange(value: string): void;
  /**
   * Called once each time the code reaches six digits by typing, pasting or
   * the platform's autofill, so the step goes on without a press of the button.
   */
  onComplete?(code: string): void;
  disabled?: boolean;
  invalid?: boolean;
  errorId?: string;
  autoFocus?: boolean;
  ref?: Ref<HTMLInputElement>;
}

/** The digits of what was typed or pasted, "482 913" and "482-913" alike, up to six. */
export function codeDigits(typed: string): string {
  return typed.replace(/\D/g, "").slice(0, CODE_LENGTH);
}

/**
 * Six cells for a one-time code, drawn over a single input. The input keeps
 * what one field gives for free: the phone's code suggestion from a text
 * message or a password manager (`one-time-code`), a paste that fills every
 * cell, and one label for a screen reader.
 *
 * The input takes no `maxLength`: a pasted "482 913" would be cut before the
 * space is dropped.
 */
export function CodeCells({
  label,
  value,
  onChange,
  onComplete,
  disabled = false,
  invalid = false,
  errorId,
  autoFocus,
  ref,
}: CodeCellsProps) {
  const id = useId();
  const current = Math.min(value.length, CODE_LENGTH - 1);

  return (
    <div className={styles.field}>
      {/* The cells say what they are by their look; the name is for a screen reader. */}
      <label className="sr-only" htmlFor={id}>
        {label}
      </label>
      <div className={styles.cells} data-invalid={invalid || undefined} data-disabled={disabled || undefined}>
        {Array.from({ length: CODE_LENGTH }, (_, index) => (
          <span key={index} className={styles.cell} data-current={index === current || undefined} aria-hidden="true">
            {value[index] ?? ""}
          </span>
        ))}
        <input
          ref={ref}
          id={id}
          name="code"
          type="text"
          inputMode="numeric"
          autoComplete="one-time-code"
          spellCheck={false}
          value={value}
          disabled={disabled}
          autoFocus={autoFocus}
          aria-invalid={invalid || undefined}
          aria-describedby={errorId}
          onChange={(event) => {
            const next = codeDigits(event.target.value);
            onChange(next);
            // From the change itself and not an effect, so a render that runs
            // twice cannot send the same code twice and spend an attempt.
            if (next.length === CODE_LENGTH && value.length < CODE_LENGTH) onComplete?.(next);
          }}
        />
      </div>
    </div>
  );
}
