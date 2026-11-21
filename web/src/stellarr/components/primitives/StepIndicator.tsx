import styles from "./StepIndicator.module.css";

/** The three bars of the mark, lit from the left as far as `lit` says. */
export function MarkBars({ lit }: { lit: number }) {
  return (
    <span className={styles.bars} aria-hidden="true">
      {[1, 2, 3].map((bar) => (
        <i key={bar} data-on={bar <= lit || undefined} />
      ))}
    </span>
  );
}

export interface StepIndicatorProps {
  /** How many of the three bars are lit: the step, or all three once done. */
  lit: number;
  /** Where the flow stands, such as "Step 2 of 3" or "Done". */
  label: string;
  /** The name of the step. */
  name: string;
}

/**
 * Where a flow of three steps stands, as the first run of an instance and
 * the create-server dialog both show it: the mark's bars, the step and its
 * name.
 */
export function StepIndicator({ lit, label, name }: StepIndicatorProps) {
  return (
    <p className={styles.steps}>
      <MarkBars lit={lit} />
      <span>
        <strong>{label}</strong> · {name}
      </span>
    </p>
  );
}
