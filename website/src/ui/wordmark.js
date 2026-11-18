const REDUCED = matchMedia('(prefers-reduced-motion: reduce)').matches;
const narrow = matchMedia('(max-width: 812px)');

// The wordmark sits beside the 3D mark and leaves letter by letter as soon as
// scrolling starts: each letter lifts, tilts and blurs out in turn.
export class Wordmark {
  constructor(el) {
    this.el = el;
    this.letters = [...el.textContent].map((ch) => {
      const span = document.createElement('span');
      span.textContent = ch;
      span.setAttribute('aria-hidden', 'true');
      return span;
    });
    el.setAttribute('aria-label', el.textContent);
    el.replaceChildren(...this.letters);
  }

  // Positions the word from the mark's projection. Without a scene there is
  // no anchor and the stylesheet centres it.
  place(anchor) {
    const style = this.el.style;
    if (narrow.matches) {
      // Stacked lockup: the wordmark centred under the mark.
      style.transform = `translate3d(${anchor.cx.toFixed(1)}px, ${(anchor.bottom + anchor.size * 0.25).toFixed(1)}px, 0) translateX(-50%)`;
      style.fontSize = `${(anchor.size * 0.62).toFixed(1)}px`;
    } else {
      style.transform = `translate3d(${anchor.x.toFixed(1)}px, ${anchor.y.toFixed(1)}px, 0) translateY(-50%)`;
      style.fontSize = `${anchor.size.toFixed(1)}px`;
    }
  }

  update(t) {
    this.el.style.visibility = t < 1.2 ? 'visible' : 'hidden';
    this.letters.forEach((l, i) => {
      const raw = Math.min(1, Math.max(0, (t - 0.03 - i * 0.04) / 0.42));
      const p = raw * raw * (3 - 2 * raw);
      l.style.opacity = (1 - p).toFixed(3);
      if (REDUCED) return;
      l.style.transform = `translate3d(0, ${(-0.55 * p).toFixed(3)}em, 0) rotate(${(-10 * p).toFixed(2)}deg)`;
      l.style.filter = p > 0.001 ? `blur(${(7 * p).toFixed(2)}px)` : 'none';
    });
  }
}
