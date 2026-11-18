// Section lengths are in scroll units. One wheel notch is 0.1 of a unit.
// The overview span also ends the wordmark that sits beside the mark.
export const SECTIONS = [
  { id: 'overview', title: 'Overview', span: 1.2 },
  { id: 'about', title: 'About', span: 1 },
  { id: 'start', title: 'Get started', span: 1 },
];

// Where a marker click or arrow key lands inside a section.
export const SETTLE = 0.3;

export const FOV = 28;

// The camera follows one continuous path. `t` is the scroll position.
// Lengths are in world units: the mark is 1.06 wide and its top sits at 1.18.
// `bend` is the ground curvature (1 / 2R). Each section reaches its pose at
// start + SETTLE and drifts until the next one begins.
const key = (t, pos, tgt, bend) => ({ t, pos, tgt, bend });

export const CAMERA = [
  // Overview: the full lockup; the wordmark lifts away and the camera turns to the mark.
  key(0, [2.03, 0.8, 8.4], [2.03, 0.72, 0], 0.02),
  key(0.6, [-0.6, 2.0, 6.9], [0.4, 0.68, 0], 0.02),
  // About: side view, mark to the right of the text.
  key(1.2, [-0.7, 1.2, 5.6], [-0.8, 0.7, 0], 0.0125),
  key(1.5, [-0.8, 1.18, 5.45], [-0.82, 0.68, 0], 0.0125),
  key(2.15, [-1.2, 1.2, 5.25], [-0.82, 0.68, 0], 0.0125),
  // Get started: rising away from the mark, which rests above the footer.
  key(2.5, [0.3, 3.6, 8.2], [0, -0.65, 0], 0.035),
  key(3.2, [0.15, 3.9, 8.8], [0, -0.65, 0], 0.035),
];
