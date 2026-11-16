import { useLayoutEffect, useRef, type ComponentType, type ReactNode } from "react";

import styles from "./AuthFrame.module.css";
import { StellarrMark } from "./StellarrMark";

export interface AuthFrameProps {
  title: string;
  description: string;
  /** A contextual action placed directly above the heading. */
  headingAction?: ReactNode;
  children: ReactNode;
}

/**
 * The element that last took the focus inside a frame. One frame is on the
 * page at a time, and a frame that replaces another reads what the old one
 * held.
 */
let focusedInFrame: Element | null = null;

/** Whether the focus went down with an element of a frame and now sits on <body>. */
function focusWasLost(): boolean {
  const active = document.activeElement;
  return (
    focusedInFrame !== null &&
    !focusedInFrame.isConnected &&
    (active === null || active === document.body)
  );
}

/**
 * The shell every account screen sits in: one centred column on the canvas.
 *
 * The lockup carries the brand, so the heading names the task and the
 * description gets a single line to say why. A second large line competing
 * with the heading is what made these screens read as noise.
 *
 * Steps swap what the frame holds: a form turns into its confirmation, a
 * button leads to the next step, a frame stands in until the server answers.
 * When the element that had the focus goes with them, the focus would fall
 * to <body> and Tab would start over at the top of the document, so the
 * frame gives it to its heading, from where reading and Tab go on into the
 * new content. A frame never takes the focus otherwise, so a page that was
 * just opened keeps the browser's start.
 */
export function AuthFrame({ title, description, headingAction, children }: AuthFrameProps) {
  const heading = useRef<HTMLHeadingElement>(null);

  // After every render: the frame's content may have changed, or the frame
  // may have just replaced another.
  useLayoutEffect(() => {
    if (!focusWasLost()) return;
    focusedInFrame = null;
    heading.current?.focus({ preventScroll: true });
  });

  return (
    <main
      className={styles.screen}
      onFocusCapture={(event) => {
        focusedInFrame = event.target;
      }}
    >
      <div className={styles.column}>
        <header className={styles.header}>
          <p className={styles.brand}>
            <StellarrMark />
            <span>Stellarr</span>
          </p>
          <div className={styles.titles}>
            {headingAction}
            <h1 ref={heading} className={styles.heading} tabIndex={-1}>
              {title}
            </h1>
            <p className={styles.description}>{description}</p>
          </div>
        </header>
        {children}
      </div>
    </main>
  );
}

export { styles as authStyles };

/**
 * What a sign-in step is drawn inside: the full sign-in page by default, or a
 * dialog when the same step runs from the account settings.
 */
export type StepFrame = ComponentType<{ title: string; description: string; children: ReactNode }>;
