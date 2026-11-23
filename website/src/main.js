import './styles.css';

const reducedMotion = window.matchMedia('(prefers-reduced-motion: reduce)');

function storageGet(key) {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

function storageSet(key, value) {
  try {
    localStorage.setItem(key, value);
  } catch {
    // Private windows and blocked storage only lose the remembered choice.
  }
}

function initAnnouncement() {
  const bar = document.getElementById('announce');
  if (storageGet('stellarr-announce-hidden') === '1') bar.hidden = true;
  bar.querySelector('.announce__close').addEventListener('click', () => {
    bar.hidden = true;
    storageSet('stellarr-announce-hidden', '1');
  });
}

function initDrawer() {
  const drawer = document.getElementById('drawer');
  const opener = document.querySelector('.nav .menu-button');
  const setOpen = (open) => {
    drawer.classList.toggle('is-open', open);
    drawer.setAttribute('aria-hidden', String(!open));
    opener.setAttribute('aria-expanded', String(open));
    document.body.style.overflow = open ? 'hidden' : '';
    if (open) drawer.querySelector('[data-close]').focus();
    else opener.focus({ preventScroll: true });
  };
  opener.addEventListener('click', () => setOpen(true));
  drawer.querySelectorAll('[data-close]').forEach((el) => el.addEventListener('click', () => setOpen(false)));
  document.addEventListener('keydown', (event) => {
    if (event.key === 'Escape' && drawer.classList.contains('is-open')) setOpen(false);
  });
}

// A star field drawn on a canvas, with a shooting star every few seconds. The
// sparks and the app window lean a little with the pointer and the scroll.
class Sky {
  static colors = ['243,244,246', '243,244,246', '243,244,246', '201,196,255', '186,206,255'];

  constructor(section) {
    this.section = section;
    this.canvas = section.querySelector('.sky__stars');
    this.ctx = this.canvas.getContext('2d');
    this.sparks = [...section.querySelectorAll('.spark')];
    this.window = section.querySelector('.window');
    this.pointer = { x: 0, y: 0 };
    this.eased = { x: 0, y: 0 };
    this.stars = [];
    this.shooting = null;
    this.nextShooting = 0;
    this.visible = true;
    this.running = false;
    this.frame = this.frame.bind(this);

    window.addEventListener(
      'pointermove',
      (event) => {
        if (event.pointerType && event.pointerType !== 'mouse') return;
        this.pointer.x = (event.clientX / window.innerWidth) * 2 - 1;
        this.pointer.y = (event.clientY / window.innerHeight) * 2 - 1;
      },
      { passive: true },
    );
    new IntersectionObserver(([entry]) => {
      this.visible = entry.isIntersecting;
      this.update();
    }).observe(section);
    let resizeTimer;
    window.addEventListener('resize', () => {
      clearTimeout(resizeTimer);
      resizeTimer = setTimeout(() => this.reset(), 150);
    });
    this.reset();
  }

  get moving() {
    return (
      this.visible &&
      !document.hidden &&
      !reducedMotion.matches &&
      !document.documentElement.classList.contains('motion-paused')
    );
  }

  reset() {
    const { width, height } = this.section.getBoundingClientRect();
    const ratio = Math.min(window.devicePixelRatio || 1, 2);
    this.width = width;
    this.height = height;
    this.canvas.width = Math.round(width * ratio);
    this.canvas.height = Math.round(height * ratio);
    this.ctx.setTransform(ratio, 0, 0, ratio, 0, 0);
    const random = (min, max) => min + Math.random() * (max - min);
    this.stars = Array.from({ length: Math.min(Math.round((width * height) / 2600), 460) }, () => {
      const layer = Math.random() < 0.6 ? 0 : Math.random() < 0.7 ? 1 : 2;
      return {
        x: Math.random() * width,
        y: Math.random() * height,
        layer,
        radius: [random(0.35, 0.8), random(0.7, 1.2), random(1.1, 1.7)][layer],
        alpha: layer === 0 ? random(0.25, 0.6) : random(0.5, 0.95),
        speed: random(0.6, 2.2),
        phase: Math.random() * Math.PI * 2,
        color: Sky.colors[Math.floor(Math.random() * Sky.colors.length)],
        cross: layer === 2 && Math.random() < 0.35,
      };
    });
    this.drawStill();
    this.update();
  }

  update() {
    if (this.moving && !this.running) {
      this.running = true;
      this.nextShooting = performance.now() / 1000 + 2.5;
      requestAnimationFrame(this.frame);
    } else if (!this.moving && this.running) {
      this.running = false;
      this.drawStill();
    }
  }

  drawStill() {
    this.ctx.clearRect(0, 0, this.width, this.height);
    for (const star of this.stars) this.drawStar(star, 1, 0, 0);
    this.sparks.forEach((spark) => (spark.style.transform = ''));
    this.window?.style.removeProperty('--tilt-x');
    this.window?.style.removeProperty('--tilt-y');
  }

  drawStar(star, twinkle, offsetX, offsetY) {
    const { width, height, ctx } = this;
    const x = (((star.x + offsetX * (star.layer + 1)) % width) + width) % width;
    const y = (((star.y + offsetY * (star.layer + 1)) % height) + height) % height;
    // Stars fade out towards the bottom, where the sky turns indigo.
    const alpha = star.alpha * twinkle * Math.max(0, Math.min(1, 1.15 - y / height));
    if (alpha <= 0.01) return;
    ctx.fillStyle = `rgba(${star.color},${alpha.toFixed(3)})`;
    ctx.beginPath();
    ctx.arc(x, y, star.radius, 0, Math.PI * 2);
    ctx.fill();
    if (!star.cross) return;
    const arm = star.radius * 5 * twinkle;
    ctx.strokeStyle = `rgba(${star.color},${(alpha * 0.7).toFixed(3)})`;
    ctx.lineWidth = 0.8;
    ctx.beginPath();
    ctx.moveTo(x - arm, y);
    ctx.lineTo(x + arm, y);
    ctx.moveTo(x, y - arm);
    ctx.lineTo(x, y + arm);
    ctx.stroke();
  }

  drawShootingStar(time) {
    const { width, height, ctx } = this;
    const random = (min, max) => min + Math.random() * (max - min);
    if (!this.shooting) {
      if (time > this.nextShooting) {
        this.shooting = {
          x: random(width * 0.35, width * 1.05),
          y: random(-20, height * 0.25),
          vx: -random(520, 760),
          vy: random(220, 320),
          age: 0,
          life: random(0.8, 1.2),
        };
      }
      return;
    }
    const star = this.shooting;
    star.age += 1 / 60;
    const progress = star.age / star.life;
    if (progress >= 1) {
      this.shooting = null;
      this.nextShooting = time + random(4, 9);
      return;
    }
    const x = star.x + star.vx * star.age;
    const y = star.y + star.vy * star.age;
    const length = Math.hypot(star.vx, star.vy);
    const tailX = x - (star.vx / length) * 140;
    const tailY = y - (star.vy / length) * 140;
    const gradient = ctx.createLinearGradient(x, y, tailX, tailY);
    gradient.addColorStop(0, `rgba(243,244,246,${(0.9 * Math.sin(progress * Math.PI)).toFixed(3)})`);
    gradient.addColorStop(1, 'rgba(201,196,255,0)');
    ctx.strokeStyle = gradient;
    ctx.lineWidth = 1.6;
    ctx.lineCap = 'round';
    ctx.beginPath();
    ctx.moveTo(x, y);
    ctx.lineTo(tailX, tailY);
    ctx.stroke();
  }

  frame(now) {
    if (!this.running) return;
    const time = now / 1000;
    const { eased, pointer } = this;
    eased.x += (pointer.x - eased.x) * 0.06;
    eased.y += (pointer.y - eased.y) * 0.06;
    const scrolled = Math.max(-this.height, Math.min(this.height, -this.section.getBoundingClientRect().top));
    this.ctx.clearRect(0, 0, this.width, this.height);
    for (const star of this.stars) {
      this.drawStar(
        star,
        0.55 + 0.45 * Math.sin(time * star.speed + star.phase),
        eased.x * -6,
        eased.y * -4 - scrolled * 0.04,
      );
    }
    this.drawShootingStar(time);
    if (this.window) {
      this.window.style.setProperty('--tilt-x', `${(eased.x * 4).toFixed(2)}deg`);
      this.window.style.setProperty('--tilt-y', `${(eased.y * -3).toFixed(2)}deg`);
    }
    for (const spark of this.sparks) {
      const depth = Number(spark.dataset.depth) || 0;
      spark.style.transform = `translate3d(${(eased.x * -22 * depth).toFixed(2)}px,${(eased.y * -16 * depth - scrolled * 0.18 * depth).toFixed(2)}px,0)`;
    }
    requestAnimationFrame(this.frame);
  }
}

// The pause button stops the skies, the sparks and the tour film in the hero.
function initMotion() {
  const skies = [...document.querySelectorAll('[data-sky]')].map((section) => new Sky(section));
  const toggle = document.querySelector('.motion-toggle');
  const film = document.querySelector('.hero .window img');
  const still = document.querySelector('.hero .window source').getAttribute('srcset');
  const moving = film.getAttribute('src');
  const root = document.documentElement;

  const apply = (paused) => {
    root.classList.toggle('motion-paused', paused);
    toggle.setAttribute('aria-pressed', String(paused));
    toggle.setAttribute('aria-label', paused ? 'Play animations' : 'Pause animations');
    film.src = paused ? still : moving;
    skies.forEach((sky) => sky.update());
  };
  toggle.addEventListener('click', () => {
    const paused = !root.classList.contains('motion-paused');
    storageSet('stellarr-motion', paused ? 'paused' : 'playing');
    apply(paused);
  });
  document.addEventListener('visibilitychange', () => skies.forEach((sky) => sky.update()));
  reducedMotion.addEventListener('change', () => skies.forEach((sky) => sky.reset()));
  apply(storageGet('stellarr-motion') === 'paused');
}

function initCallPanel() {
  const root = document.getElementById('call-demo');
  const spec = root.querySelector('.share__spec');
  const hint = root.querySelector('.panel__hint');

  root.querySelectorAll('[role="radiogroup"]').forEach((group) => {
    const options = [...group.querySelectorAll('[role="radio"]')];
    options.forEach((option) =>
      option.addEventListener('click', () => {
        options.forEach((other) => other.setAttribute('aria-checked', String(other === option)));
        if (option.dataset.spec) spec.textContent = option.dataset.spec;
        if (option.dataset.hint) hint.textContent = option.dataset.hint;
      }),
    );
  });

  const tabs = [...root.querySelectorAll('[role="tab"]')];
  const show = (tab) => {
    for (const other of tabs) {
      const selected = other === tab;
      other.setAttribute('aria-selected', String(selected));
      other.tabIndex = selected ? 0 : -1;
      document.getElementById(other.getAttribute('aria-controls')).hidden = !selected;
    }
    for (const stage of root.querySelectorAll('.call__stage > [data-stage]'))
      stage.hidden = stage.dataset.stage !== tab.dataset.stage;
  };
  tabs.forEach((tab, index) => {
    tab.addEventListener('click', () => show(tab));
    tab.addEventListener('keydown', (event) => {
      if (event.key !== 'ArrowRight' && event.key !== 'ArrowLeft') return;
      const next = tabs[(index + (event.key === 'ArrowRight' ? 1 : tabs.length - 1)) % tabs.length];
      show(next);
      next.focus();
    });
  });
  show(tabs[0]);
}

// Each message switches between its original and its translation, with the
// same Translate and Show original actions as the app.
function initTranslation() {
  for (const message of document.querySelectorAll('#translate-demo .msg')) {
    const text = message.querySelector('.msg__text');
    const action = message.querySelector('.msg__action');
    const original = text.dataset.original;
    action.addEventListener('click', () => {
      const showing = text.lang === 'en' ? 'pl' : 'en';
      text.lang = showing;
      text.textContent = text.dataset[showing];
      action.textContent = showing === original ? 'Translate' : 'Show original';
    });
  }
}

function initCopy() {
  const button = document.getElementById('copy');
  const label = button.querySelector('span');
  const icon = button.querySelector('use');
  const command = document.querySelector('.command code').lastChild.textContent.trim();
  let timer;
  const flash = (copied) => {
    clearTimeout(timer);
    label.textContent = copied ? 'Copied' : 'Could not copy';
    if (copied) icon.setAttribute('href', '#i-check');
    timer = setTimeout(() => {
      label.textContent = 'Copy';
      icon.setAttribute('href', '#i-copy');
    }, 2000);
  };
  button.addEventListener('click', async () => {
    try {
      await navigator.clipboard.writeText(command);
      flash(true);
    } catch {
      flash(false);
    }
  });
}

initAnnouncement();
initDrawer();
initMotion();
initCallPanel();
initTranslation();
initCopy();
