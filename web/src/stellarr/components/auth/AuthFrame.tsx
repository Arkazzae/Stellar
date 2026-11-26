import { useLayoutEffect, useRef, type ComponentType, type ReactNode } from "react";

import { Sticker } from "@/components/primitives/MascotArt";
import { useMediaQuery } from "@/hooks/useMediaQuery";

import styles from "./AuthFrame.module.css";
import { SceneImage, sceneArt, stickerArt, type AuthScene, type OutcomeSticker } from "./Mascot";

export type { AuthScene, OutcomeSticker } from "./Mascot";

/** From this width the scene stands beside the form; AuthFrame.module.css uses the same. */
const WIDE = "(min-width: 960px)";

export interface AuthFrameProps {
  title: string;
  description: ReactNode;
  /** The moment the page stands for: a scene beside the form, or a sticker on a narrow screen. */
  scene?: AuthScene;
  /** An outcome page's sticker, shown at every width instead of a scene. */
  outcome?: OutcomeSticker;
  /** A contextual line or action placed directly above the heading. */
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

/** The Stellarr mark and wordmark, drawn from `website/src/images/logo/small.svg`, the mark for 16–47px. */
export function StellarrLockup() {
  return (
    <p className={styles.lockup}>
      <svg width={32} height={32} viewBox="0 0 64 64" aria-hidden={true} focusable="false">
        <path
          className={styles.bubble}
          d="M24 6H40C55 6 60 11 60 27C60 43 55 48 40 48H34C32.4 48 30.9 48.5 29.6 49.4L18.64 57.3C17.1 58.4 15.5 57.6 15.5 55.7V49.6C15.5 47.8 14.5 46.8 12.71 45.95C7.11 43.28 4 37.75 4 27C4 11 9 6 24 6Z"
        />
        <g className={styles.bars}>
          <rect x="15.5" y="20" width="7.5" height="14" rx="3.75" />
          <rect x="28.25" y="15" width="7.5" height="24" rx="3.75" />
          <rect x="41" y="18" width="7.5" height="18" rx="3.75" />
        </g>
      </svg>
      <span>Stellarr</span>
    </p>
  );
}

/**
 * The page every account screen is: the lockup, the heading that names the
 * task, one line that says why, and the content. A scene of the mascot
 * stands beside it on a wide screen and a sticker above the heading on a
 * narrow one; only the one the layout shows is rendered, so only that one is
 * fetched.
 *
 * Steps swap what the frame holds: a form turns into its confirmation, a
 * button leads to the next step, a frame stands in until the server answers.
 * When the element that had the focus goes with them, the focus would fall
 * to <body> and Tab would start over at the top of the document, so the
 * frame gives it to its heading, from where reading and Tab go on into the
 * new content. A frame never takes the focus otherwise, so a page that was
 * just opened keeps the browser's start.
 */
export function AuthFrame({ title, description, scene, outcome, headingAction, children }: AuthFrameProps) {
  const heading = useRef<HTMLHeadingElement>(null);
  const wide = useMediaQuery(WIDE, true);

  // After every render: the frame's content may have changed, or the frame
  // may have just replaced another.
  useLayoutEffect(() => {
    if (!focusWasLost()) return;
    focusedInFrame = null;
    heading.current?.focus({ preventScroll: true });
  });

  const shownScene = outcome === undefined && wide ? scene : undefined;
  const sticker =
    outcome !== undefined
      ? stickerArt({ outcome })
      : scene !== undefined && !wide
        ? stickerArt({ scene })
        : null;

  const column = (
    <div className={styles.column}>
      <header className={styles.header}>
        <StellarrLockup />
        {sticker === null ? null : <Sticker name={sticker} size="page" rise />}
        <div className={styles.titles}>
          {headingAction}
          <h1 ref={heading} className={styles.heading} tabIndex={-1}>
            {title}
          </h1>
          {description ? <p className={styles.description}>{description}</p> : null}
        </div>
      </header>
      {children}
    </div>
  );

  return (
    <main
      className={styles.screen}
      onFocusCapture={(event) => {
        focusedInFrame = event.target;
      }}
    >
      {/* One wrapper either way, so crossing the breakpoint keeps the form
          and what was typed into it. */}
      <div className={shownScene === undefined ? styles.alone : styles.pair}>
        {column}
        {shownScene === undefined ? null : (
          <div className={styles.art}>
            <SceneImage name={sceneArt(shownScene)} />
          </div>
        )}
      </div>
    </main>
  );
}

/**
 * What a sign-in step is drawn inside: the full sign-in page by default, or a
 * dialog when the same step runs from the account settings, which shows no
 * mascot and ignores `scene`.
 */
export type StepFrame = ComponentType<{
  title: string;
  description: string;
  scene?: AuthScene;
  children: ReactNode;
}>;
