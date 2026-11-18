import { SECTIONS, SETTLE } from '../config.js';

const clamp = (v, a, b) => Math.min(b, Math.max(a, v));

// Virtual scroll: input moves a target and the position chases it, so the
// wheel scrubs one continuous camera path rather than moving the document.
export class Scroller {
  constructor() {
    this.starts = [];
    let sum = 0;
    for (const s of SECTIONS) {
      this.starts.push(sum);
      sum += s.span;
    }
    this.total = sum;
    this.target = 0;
    this.current = 0;
    this.touchY = null;

    addEventListener('wheel', (e) => this.onWheel(e), { passive: false });
    addEventListener('touchstart', (e) => (this.touchY = e.touches[0].clientY), { passive: true });
    addEventListener('touchmove', (e) => this.onTouch(e), { passive: false });
    addEventListener('touchend', () => (this.touchY = null), { passive: true });
    addEventListener('keydown', (e) => this.onKey(e));
  }

  // Index of the section the camera is in. Sections never overlap.
  get index() {
    return this.indexAt(this.current);
  }

  indexAt(t) {
    for (let i = SECTIONS.length - 1; i >= 0; i--) if (t >= this.starts[i]) return i;
    return 0;
  }

  move(by) {
    this.target = clamp(this.target + by, 0, this.total);
  }

  goTo(i) {
    const s = clamp(i, 0, SECTIONS.length - 1);
    this.target = s === 0 ? 0 : this.starts[s] + SETTLE;
  }

  onWheel(e) {
    e.preventDefault();
    const unit = e.deltaMode === 1 ? 16 : e.deltaMode === 2 ? innerHeight : 1;
    this.move(clamp(((e.deltaY * unit) / 120) * 0.1, -1, 1));
  }

  onTouch(e) {
    e.preventDefault();
    if (this.touchY === null) return;
    const y = e.touches[0].clientY;
    this.move(((this.touchY - y) / innerHeight) * 2.4);
    this.touchY = y;
  }

  onKey(e) {
    if (e.target.closest?.('input, textarea')) return;
    const i = this.indexAt(this.target);
    if (e.key === 'ArrowDown' || e.key === 'PageDown' || (e.key === ' ' && !e.target.closest?.('button'))) {
      e.preventDefault();
      this.goTo(i + 1);
    } else if (e.key === 'ArrowUp' || e.key === 'PageUp') {
      e.preventDefault();
      this.goTo(i - 1);
    } else if (e.key === 'Home') {
      e.preventDefault();
      this.goTo(0);
    } else if (e.key === 'End') {
      e.preventDefault();
      this.goTo(SECTIONS.length - 1);
    }
  }

  update(dt) {
    this.current += (this.target - this.current) * (1 - Math.exp(-dt * 5));
    if (Math.abs(this.target - this.current) < 1e-4) this.current = this.target;
  }
}
