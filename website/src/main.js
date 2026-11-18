import './styles.css';
import { Scroller } from './ui/scroller.js';
import { Chrome } from './ui/chrome.js';
import { Wordmark } from './ui/wordmark.js';
import { initCopy } from './ui/copy.js';
import { Stage } from './gl/stage.js';

const preloader = document.getElementById('preloader');
const bar = document.querySelector('#preloader-bar div');
const progress = (v) => (bar.style.transform = `scaleX(${v})`);
const nextPaint = () => new Promise((r) => requestAnimationFrame(() => setTimeout(r, 0)));

const scroller = new Scroller();
const chrome = new Chrome(scroller);
const wordmark = new Wordmark(document.querySelector('.hero-word'));
initCopy();

// Without WebGL 2 the page still scrolls through its sections, just without the scene.
function fallback() {
  document.documentElement.classList.add('no-scene');
  document.getElementById('nogl').hidden = false;
  preloader.classList.add('is-done');
  let last = performance.now();
  const frame = (now) => {
    const dt = Math.min(0.05, (now - last) / 1000);
    last = now;
    scroller.update(dt);
    chrome.update(dt);
    wordmark.update(scroller.current);
    requestAnimationFrame(frame);
  };
  requestAnimationFrame(frame);
}

async function start() {
  progress(0.15);
  await nextPaint();

  let stage;
  try {
    stage = new Stage(document.getElementById('gl'));
  } catch (err) {
    console.error(err);
    fallback();
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
    chrome.update(dt);
    stage.update({ t: scroller.current, time: now / 1000, dt });
    wordmark.place(stage.anchor);
    wordmark.update(scroller.current);
    requestAnimationFrame(frame);
  };
  requestAnimationFrame(frame);

  preloader.classList.add('is-done');
  setTimeout(() => preloader.remove(), 900);
}

start();
