import { createContext, useContext } from "react";

import type { ConfirmSubject } from "./ConfirmSubject";

/*
 * The question card's contract, apart from the card.
 *
 * The providers the first page loads ask questions too (encryption does), so
 * what they need, the hooks and the shape of a question, lives here on its
 * own. The card that draws a question, and the provider that holds one, load
 * with the signed-in app and stay out of the first page.
 */

/** One line of text the deed needs before it can be done. */
export interface ConfirmField {
  /** The field's own name, shown above it in sentence case. */
  label: string;
  placeholder?: string;
  maxLength?: number;
  /**
   * Empty is a valid answer, and the label says so. Without this the deed's
   * button stays disabled until something is typed.
   */
  optional?: boolean;
  /**
   * The exact words that answer it, such as the name of what is about to be
   * deleted. The deed's button stays disabled until they are typed, letter
   * case included, and a tick at the end of the field says they are.
   */
  match?: string;
  /** A sentence under the field: where the answer will be seen. */
  hint?: string;
}

/** What the card says and offers: everything but the answering. */
export interface ConfirmQuestion {
  /** The question itself: short, and it ends in a question mark. */
  title: string;
  /** What the question does not say, usually the consequence. */
  description?: string;
  /**
   * The one thing the question is about, drawn as a row under the sentence.
   * The title never names it, because a name inflected into a Polish sentence
   * comes out wrong.
   */
  subject?: ConfirmSubject;
  /** Names the deed ("Delete", "Block"), never "OK". */
  confirmLabel?: string;
  cancelLabel?: string;
  /** The deed destroys something: the button is red. */
  danger?: boolean;
  /** Turns the question into one that also collects an answer in words. */
  field?: ConfirmField;
  /**
   * Nothing can be done yet, so the card has one answer, which closes it.
   * Its label is `cancelLabel`, "Zamknij" unless given.
   */
  notice?: boolean;
  /** What the deed's button says while it runs ("Usuwanie…"). */
  busyLabel?: string;
}

/** A question, and the deed it may run; answering is the provider's half. */
export interface ConfirmRequest extends ConfirmQuestion {
  /**
   * The deed itself, given the typed answer. With it the card stays open while
   * the deed runs and closes only once it has succeeded; a rejection keeps the
   * card open with `failure` above the answers, and the answers work again.
   * Without it the card closes on the answer and the caller acts.
   */
  run?: (answer: string) => Promise<unknown>;
  /** The error line when `run` rejects; a generic one unless given. */
  failure?: string;
}

/** A question with a line to fill in: the field is the point. */
export interface PromptRequest extends ConfirmRequest {
  field: ConfirmField;
}

/**
 * Resolves true only if the reader pressed the confirming button, and, for a
 * request with a `run`, only after the deed succeeded.
 */
export type Confirm = (request: ConfirmRequest) => Promise<boolean>;

/** Resolves the typed answer, trimmed, or null if the question was dismissed. */
export type Prompt = (request: PromptRequest) => Promise<string | null>;

export interface ConfirmValue {
  confirm: Confirm;
  prompt: Prompt;
}

/** Provided by `ConfirmProvider`, which the signed-in app mounts at its root. */
export const ConfirmContext = createContext<ConfirmValue | null>(null);

function useConfirmContext(): ConfirmValue {
  const value = useContext(ConfirmContext);
  if (value === null) {
    throw new Error("useConfirm must be used inside <ConfirmProvider>");
  }
  return value;
}

export function useConfirm(): Confirm {
  return useConfirmContext().confirm;
}

export function usePrompt(): Prompt {
  return useConfirmContext().prompt;
}
