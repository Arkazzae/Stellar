export interface ProgressRingProps {
  /** Whole percent received, or null while the total is unknown. */
  percent: number | null;
  /** The outer diameter in px. */
  size: number;
  /** In px; the ring stays this thick at any size. */
  stroke?: number;
  className?: string;
}

/**
 * A download on its way, as a ring that fills clockwise from twelve o'clock.
 *
 * Decorative: the control it sits on says the same in words and carries
 * `aria-busy`. An unknown amount is a short arc that turns, or a broken ring
 * that stays still where motion is reduced, so it never passes for a share.
 */
export function ProgressRing({ percent, size, stroke = 2, className }: ProgressRingProps) {
  const centre = size / 2;
  const radius = centre - stroke / 2;
  const known = percent !== null;
  const classes = ["progress-ring", known ? null : "unknown", className].filter(Boolean).join(" ");

  return (
    <svg
      className={classes}
      width={size}
      height={size}
      viewBox={`0 0 ${size} ${size}`}
      aria-hidden="true"
      focusable="false"
    >
      <circle className="progress-ring-track" cx={centre} cy={centre} r={radius} strokeWidth={stroke} />
      <circle
        className="progress-ring-fill"
        cx={centre}
        cy={centre}
        r={radius}
        strokeWidth={stroke}
        pathLength={100}
        transform={`rotate(-90 ${centre} ${centre})`}
        // An unknown amount is drawn by the stylesheet, which knows whether
        // motion is reduced.
        style={known ? { strokeDasharray: `${Math.min(100, Math.max(0, percent))} 100` } : undefined}
      />
    </svg>
  );
}
