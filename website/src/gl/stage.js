import * as THREE from 'three';
import { EffectComposer } from 'three/addons/postprocessing/EffectComposer.js';
import { RenderPass } from 'three/addons/postprocessing/RenderPass.js';
import { UnrealBloomPass } from 'three/addons/postprocessing/UnrealBloomPass.js';
import { OutputPass } from 'three/addons/postprocessing/OutputPass.js';
import { ShaderPass } from 'three/addons/postprocessing/ShaderPass.js';
import { CAMERA, FOV } from '../config.js';
import { Mark, FOOTPRINT, LOCKUP } from './mark.js';
import { Floor } from './floor.js';

const COARSE = matchMedia('(pointer: coarse)').matches;
const REDUCED = matchMedia('(prefers-reduced-motion: reduce)').matches;

// Grain, vignette and a lifted black, applied after tone mapping.
const FinishShader = {
  uniforms: {
    tDiffuse: { value: null },
    uTime: { value: 0 },
    uAspect: { value: 1 },
  },
  vertexShader: `varying vec2 vUv; void main(){ vUv = uv; gl_Position = vec4(position.xy, 0.0, 1.0); }`,
  fragmentShader: /* glsl */ `
    precision highp float;
    uniform sampler2D tDiffuse;
    uniform float uTime, uAspect;
    varying vec2 vUv;
    float hash(vec2 p) {
      vec3 q = fract(vec3(p.xyx) * 0.1031);
      q += dot(q, q.yzx + 33.33);
      return fract((q.x + q.y) * q.z);
    }
    float blot(vec2 p) {
      vec2 i = floor(p); vec2 f = fract(p); f = f * f * (3.0 - 2.0 * f);
      return mix(mix(hash(i), hash(i + vec2(1, 0)), f.x), mix(hash(i + vec2(0, 1)), hash(i + vec2(1, 1)), f.x), f.y);
    }
    void main() {
      vec3 col = texture2D(tDiffuse, vUv).rgb;
      vec2 q = (vUv - 0.5) * vec2(uAspect, 1.0) / max(uAspect, 1.0);
      col *= 1.0 - dot(q, q) * 1.15;
      col += vec3(0.031, 0.031, 0.033);

      // Very faint colour drift, visible only in the shadows.
      vec2 b = vUv * vec2(uAspect, 1.0) * 1.7;
      float t = uTime * 0.05;
      col.r += (blot(b + t) - 0.5) * 0.012;
      col.b += (blot(b * 1.2 - t + 7.0) - 0.5) * 0.012;

      float tick = floor(uTime * 24.0);
      float grain = hash(gl_FragCoord.xy + tick * 17.31) - 0.5;
      col += grain * 0.05;
      gl_FragColor = vec4(col, 1.0);
    }
  `,
};

// Monotone cubic (Fritsch-Butland) tangents per channel, so a fast move into
// a key never carries the camera past it during the slow hold that follows.
const CHANNELS = [
  ...[0, 1, 2].map((n) => (k) => k.pos[n]),
  ...[0, 1, 2].map((n) => (k) => k.tgt[n]),
];

const TANGENTS = CHANNELS.map((get) => {
  const K = CAMERA;
  const slope = (i) => (get(K[i + 1]) - get(K[i])) / (K[i + 1].t - K[i].t);
  return K.map((_, i) => {
    if (i === 0) return slope(0);
    if (i === K.length - 1) return slope(i - 1);
    const d0 = slope(i - 1);
    const d1 = slope(i);
    if (d0 * d1 <= 0) return 0;
    const h0 = K[i].t - K[i - 1].t;
    const h1 = K[i + 1].t - K[i].t;
    const w0 = 2 * h1 + h0;
    const w1 = h1 + 2 * h0;
    return (w0 + w1) / (w0 / d0 + w1 / d1);
  });
});

function sampleCamera(t) {
  const K = CAMERA;
  const last = K.length - 1;
  t = Math.min(K[last].t, Math.max(K[0].t, t));
  let i = 0;
  while (i < last - 1 && t > K[i + 1].t) i++;
  const a = K[i];
  const b = K[i + 1];
  const h = b.t - a.t;
  const u = (t - a.t) / h;
  const u2 = u * u;
  const u3 = u2 * u;

  const v = CHANNELS.map((get, c) => {
    const m = TANGENTS[c];
    return (
      (2 * u3 - 3 * u2 + 1) * get(a) +
      (u3 - 2 * u2 + u) * h * m[i] +
      (-2 * u3 + 3 * u2) * get(b) +
      (u3 - u2) * h * m[i + 1]
    );
  });

  return {
    pos: v.slice(0, 3),
    tgt: v.slice(3, 6),
    bend: a.bend + (b.bend - a.bend) * u * u * (3 - 2 * u),
  };
}

export class Stage {
  constructor(canvas) {
    const renderer = new THREE.WebGLRenderer({ canvas, antialias: false, powerPreference: 'high-performance' });
    if (!renderer.capabilities.isWebGL2) throw new Error('WebGL 2 is required');
    this.renderer = renderer;
    renderer.toneMapping = THREE.ACESFilmicToneMapping;
    renderer.toneMappingExposure = 1;

    this.dpr = Math.min(devicePixelRatio || 1, COARSE ? 1.5 : 2);
    this.slowFrames = 0;
    renderer.setPixelRatio(this.dpr);
    renderer.setSize(innerWidth, innerHeight, false);

    this.scene = new THREE.Scene();
    this.camera = new THREE.PerspectiveCamera(FOV, innerWidth / innerHeight, 0.05, 140);

    this.scene.environment = this.buildEnvironment();

    this.mark = new Mark();
    this.scene.add(this.mark.group);
    this.anchor = { x: 0, y: 0, size: 0 };
    this.probe = new THREE.Vector3();

    this.floor = new Floor(COARSE ? 150000 : 520000, this.mark.accent);
    this.floor.uniforms.uFoot.value.set(...FOOTPRINT);
    this.scene.add(this.floor.group);

    const size = renderer.getDrawingBufferSize(new THREE.Vector2());
    const target = new THREE.WebGLRenderTarget(size.x, size.y, { type: THREE.HalfFloatType, samples: COARSE ? 0 : 4 });
    this.composer = new EffectComposer(renderer, target);
    this.composer.setPixelRatio(this.dpr);
    this.composer.addPass(new RenderPass(this.scene, this.camera));
    this.bloom = new UnrealBloomPass(new THREE.Vector2(size.x, size.y), 0.42, 0.6, 0.55);
    this.composer.addPass(this.bloom);
    this.composer.addPass(new OutputPass());
    this.finish = new ShaderPass(FinishShader);
    this.composer.addPass(this.finish);

    this.mouse = new THREE.Vector2();
    this.mouseTarget = new THREE.Vector2();
    if (!COARSE && !REDUCED) {
      addEventListener('mousemove', (e) => {
        this.mouseTarget.set((e.clientX / innerWidth) * 2 - 1, (e.clientY / innerHeight) * 2 - 1);
      });
    }
    addEventListener('resize', () => this.resize());
    this.resize();
  }

  // Studio lighting built from emissive cards, so no HDRI has to ship.
  buildEnvironment() {
    const room = new THREE.Scene();
    room.background = new THREE.Color(0x020203);
    const card = (w, h, pos, power, color = 0xffffff) => {
      const m = new THREE.Mesh(
        new THREE.PlaneGeometry(w, h),
        new THREE.MeshBasicMaterial({ color: new THREE.Color(color).multiplyScalar(power), side: THREE.DoubleSide }),
      );
      m.position.set(...pos);
      m.lookAt(0, 0, 0);
      room.add(m);
    };
    card(7, 1.4, [0, 7.5, -5.5], 22);
    card(6, 3, [0, 8, 1.6], 9);
    card(0.8, 9, [-8.5, 2.2, -2.4], 6);
    card(0.8, 9, [8.5, 2.2, -1.2], 4, 0xeef1ff);
    card(12, 3, [0, -1.4, 9], 0.1);
    card(4, 4, [-3.5, 5, 7], 1.1);
    // A broad, faint card behind the camera so the flat face of the mark
    // shows a gradient instead of reflecting pure black.
    card(14, 7, [1.5, 3.2, 10], 0.32);
    const pmrem = new THREE.PMREMGenerator(this.renderer);
    const texture = pmrem.fromScene(room, 0.02).texture;
    pmrem.dispose();
    return texture;
  }

  resize() {
    const w = innerWidth;
    const h = innerHeight;
    this.camera.aspect = w / h;
    this.camera.updateProjectionMatrix();
    this.renderer.setPixelRatio(this.dpr);
    this.renderer.setSize(w, h, false);
    this.composer.setPixelRatio(this.dpr);
    this.composer.setSize(w, h);
    this.finish.uniforms.uAspect.value = w / h;
    this.floor.uniforms.uDpr.value = this.dpr;
  }

  // Lowers resolution when the GPU cannot hold frame rate.
  governor(dt) {
    this.slowFrames = dt > 0.03 ? this.slowFrames + 1 : Math.max(0, this.slowFrames - 1);
    if (this.slowFrames > 45 && this.dpr > 0.8) {
      this.dpr = Math.max(0.8, this.dpr * 0.82);
      this.slowFrames = 0;
      this.resize();
    }
  }

  // Where the HTML wordmark should sit, in CSS pixels: left edge, vertical
  // centre and font size, derived from the mark's projected lockup box.
  updateAnchor() {
    const g = this.mark.group.position;
    const project = (x, y) => {
      this.probe.set(x, y + g.y, LOCKUP.z).project(this.camera);
      return [(this.probe.x * 0.5 + 0.5) * innerWidth, (-this.probe.y * 0.5 + 0.5) * innerHeight];
    };
    const [x, y] = project(LOCKUP.right, LOCKUP.centreY);
    const [, top] = project(LOCKUP.right, LOCKUP.centreY + LOCKUP.half);
    const [cx, bottom] = project(0, LOCKUP.centreY - LOCKUP.half);
    const box = 2 * (y - top);
    this.anchor.x = x + (16 / 88) * box;
    this.anchor.y = y;
    this.anchor.size = (76 / 88) * box;
    // For the stacked lockup on narrow screens: centre and foot of the mark.
    this.anchor.cx = cx;
    this.anchor.bottom = bottom;
  }

  update({ t, time, dt }) {
    const pose = sampleCamera(t);
    const k = 1 - Math.exp(-dt * 3.5);
    this.mouse.lerp(this.mouseTarget, k);

    const cam = this.camera;
    // Portrait screens pull the camera back, but less than the full width
    // ratio so the mark stays readable on a phone.
    const fit = Math.max(1, 1 / cam.aspect) ** 0.8;
    const tgt = new THREE.Vector3(...pose.tgt);
    const pos = new THREE.Vector3(...pose.pos).sub(tgt).multiplyScalar(fit).add(tgt);
    // On portrait screens the wordmark stacks under the mark, so the opening
    // shot centres on the mark instead of on the side-by-side lockup.
    if (cam.aspect < 1) {
      const shift = LOCKUP.lockupCentre * (1 - THREE.MathUtils.smoothstep(t, 0, 0.6));
      pos.x -= shift;
      tgt.x -= shift;
    }
    const reach = pos.distanceTo(tgt);
    const sway = 0.012 * reach * t;
    pos.x += this.mouse.x * sway;
    pos.y -= this.mouse.y * sway * 0.6;

    cam.position.copy(pos);
    cam.lookAt(tgt);

    const u = this.floor.uniforms;
    const px = this.renderer.domElement.height / (2 * Math.tan(THREE.MathUtils.degToRad(FOV / 2)));
    u.uTime.value = REDUCED ? 0 : time;
    u.uFocus.value = reach;
    u.uPxPerRad.value = px;
    u.uIntro.value = 0.3 * (1 - THREE.MathUtils.smoothstep(t, 0.2, 1.0));

    this.updateAnchor();
    u.uBend.value = pose.bend;
    u.uPool.value = Math.max(8.5, reach * 0.95);

    this.mark.update(REDUCED ? 0 : time);
    this.finish.uniforms.uTime.value = REDUCED ? 1 : time;
    this.composer.render(dt);
    this.governor(dt);
  }
}
