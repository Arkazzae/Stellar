import './styles.css';
import { Scroller } from './ui/scroller.js';
import { Stage } from './gl/stage.js';

const preloader = document.getElementById('preloader');
const bar = document.querySelector('#preloader-bar div');
const progress = (v) => (bar.style.transform = `scaleX(${v})`);
const nextPaint = () => new Promise((r) => requestAnimationFrame(() => setTimeout(r, 0)));

const scroller = new Scroller();

async function start() {
  progress(0.15);
  await nextPaint();

  let stage;
  try {
    stage = new Stage(document.getElementById('gl'));
  } catch (err) {
    console.error(err);
    document.getElementById('nogl').hidden = false;
    preloader.classList.add('is-done');
    return;
  }

  progress(0.7);
  await Promise.race([document.fonts.ready, new Promise((r) => setTimeout(r, 2500))]);
  progress(1);
  await nextPaint();

  let last = performance.now();
  const frame = (now) => {
    const dt = Math.min(0.05, (now - last) / 1000);
    last = now;

    scroller.update(dt);
    stage.update({ t: scroller.current, time: now / 1000, dt });
    requestAnimationFrame(frame);
  };
  requestAnimationFrame(frame);

  preloader.classList.add('is-done');
  setTimeout(() => preloader.remove(), 900);
}

start();
