import { SECTIONS } from '../config.js';
import { Reveal } from './reveal.js';

const SPEED = 2; // A section takes half a second to appear or leave.

// The rail is three thin lines at the heights of the logo's meter bars
// (13, 22 and 16 units), one per section. The current one takes the accent.
const BARS = [13, 22, 16];

// Owns section visibility, the section rail and the links that move the camera.
export class Chrome {
  constructor(scroller) {
    this.scroller = scroller;
    this.rail = document.getElementById('rail');
    this.railLabel = document.getElementById('rail-label');
    this.title = -1;

    let seed = 1;
    this.pages = [...document.querySelectorAll('.page')].map((el) => ({
      el,
      ratio: 0,
      reveals: [...el.querySelectorAll('[data-fx]')].map((n) => new Reveal(n, seed++)),
    }));

    const list = document.getElementById('rail-marks');
    this.bars = SECTIONS.map((section, i) => {
      const li = document.createElement('li');
      const bar = document.createElement('button');
      bar.type = 'button';
      bar.className = 'bar';
      bar.setAttribute('aria-label', section.title);
      bar.style.setProperty('--h', (BARS[i % BARS.length] / 22).toFixed(3));
      const line = document.createElement('span');
      line.className = 'bar-line';
      bar.appendChild(line);
      bar.addEventListener('click', () => scroller.goTo(i));
      // Pointing at a line, or tabbing to it, names its section beside the rail.
      const show = () => this.setLabel(section.title);
      const hide = () => this.setLabel('');
      bar.addEventListener('pointerenter', show);
      bar.addEventListener('pointerleave', hide);
      bar.addEventListener('focus', show);
      bar.addEventListener('blur', hide);
      li.appendChild(bar);
      list.appendChild(li);
      return bar;
    });

    document.querySelector('.wordmark').addEventListener('click', (e) => {
      e.preventDefault();
      scroller.goTo(0);
    });

    // Buttons such as "Get started" move the camera to a section, named by its id.
    document.addEventListener('click', (e) => {
      const go = e.target.closest('[data-goto]');
      if (go) scroller.goTo(SECTIONS.findIndex((s) => s.id === go.dataset.goto));
    });
  }

  setLabel(text) {
    if (text) this.railLabel.textContent = text;
    this.railLabel.classList.toggle('is-in', Boolean(text));
  }

  update(dt) {
    const index = this.scroller.index;
    this.pages.forEach((p, i) => {
      const goal = i === index ? 1 : 0;
      if (p.ratio !== goal) {
        const step = dt * SPEED;
        p.ratio = goal > p.ratio ? Math.min(goal, p.ratio + step) : Math.max(goal, p.ratio - step);
        p.el.classList.toggle('is-live', p.ratio > 0 || goal === 1);
        p.el.toggleAttribute('inert', goal === 0);
        p.el.style.setProperty('--r', p.ratio.toFixed(3));
        p.reveals.forEach((r) => r.set(p.ratio));
      }
    });

    // The rail stays out of the hero and comes in once scrolling starts.
    this.rail.classList.toggle('is-on', this.scroller.current > 0.25);

    if (index !== this.title) {
      this.title = index;
      document.title = `Stellarr | ${SECTIONS[index].title}`;
      this.bars.forEach((bar, i) => {
        bar.classList.toggle('is-on', i === index);
        if (i === index) bar.setAttribute('aria-current', 'step');
        else bar.removeAttribute('aria-current');
      });
    }
  }
}
