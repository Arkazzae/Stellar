const REDUCED = matchMedia('(prefers-reduced-motion: reduce)').matches;

const rng = (seed) => () => {
  seed = (seed + 0x6d2b79f5) | 0;
  let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
  t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
  return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
};

const unmix = (a, b, x) => Math.min(1, Math.max(0, (x - a) / (b - a)));

// Whitespace that may break a line; a non-breaking space stays inside its word.
const BREAKING = /[^\S ]+/;

// Splits an element into words, each with its own timing window, so a block
// of text resolves out of blur instead of fading as one.
export class Reveal {
  constructor(el, seed) {
    this.words = [];
    const random = rng(seed * 7919 + 13);

    const sr = document.createElement('span');
    sr.className = 'sr';
    // Built from nodes, not innerText: pages start hidden, where <br> is lost.
    sr.textContent = [...el.childNodes]
      .map((n) => (n.nodeName === 'BR' ? ' ' : n.textContent))
      .join('')
      .replace(/\s+/g, ' ')
      .trim();

    const visual = document.createElement('span');
    visual.setAttribute('aria-hidden', 'true');

    const space = () => {
      const last = visual.lastChild;
      if (last && !(last.nodeType === 3 && last.textContent === ' ') && last.nodeName !== 'BR') {
        visual.appendChild(document.createTextNode(' '));
      }
    };

    for (const node of [...el.childNodes]) {
      if (node.nodeName === 'BR') {
        visual.appendChild(document.createElement('br'));
        continue;
      }
      const text = node.textContent;
      if (/^[^\S ]/.test(text)) space();
      const parts = text.split(BREAKING).filter(Boolean);
      // Inline elements such as <code> keep their tag around each word.
      const wrap = node.nodeType === 1 ? () => node.cloneNode(false) : null;
      parts.forEach((word, i) => {
        const span = document.createElement('span');
        span.className = 'w';
        if (wrap) {
          const inner = wrap();
          inner.textContent = word;
          span.appendChild(inner);
        } else {
          span.textContent = word;
        }
        visual.appendChild(span);
        if (i < parts.length - 1) visual.appendChild(document.createTextNode(' '));
        this.words.push({ span, a: random(), b: random() });
      });
      if (/[^\S ]$/.test(text) && parts.length) space();
    }

    el.replaceChildren(sr, visual);
    this.last = -1;
    this.set(0);
  }

  set(r) {
    if (r === this.last) return;
    this.last = r;
    for (const { span, a, b } of this.words) {
      const o = unmix(0.4 * a, 1 - 0.4 * b, r);
      span.style.opacity = o;
      span.style.visibility = o > 0 ? 'visible' : 'hidden';
      if (REDUCED) continue;
      span.style.transform = `translate3d(0, ${10 * (1 - o)}px, 0)`;
      span.style.filter = o < 1 ? `blur(${5 * (1 - o)}px)` : 'none';
    }
  }
}
