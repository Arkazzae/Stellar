import * as THREE from 'three';

const REACH = 13;

// The ground is a paraboloid whose curvature follows the camera, so a side
// view gets a near-flat horizon and a high view gets a strong arc. Its height
// is zero under the sculpture.
const SHARED = /* glsl */ `
  uniform float uBend;
  uniform vec2 uFoot;
  float groundY(vec2 p) {
    return -uBend * dot(p, p);
  }
  // 1.0 on the outline of the sculpture's footprint and growing outward.
  float footprint(vec2 p) {
    p = abs(p) / uFoot;
    p *= p;
    p *= p;
    return pow(p.x + p.y, 0.25);
  }
`;

const NOISE = /* glsl */ `
  float hash(vec2 p) {
    vec3 q = fract(vec3(p.xyx) * 0.1031);
    q += dot(q, q.yzx + 33.33);
    return fract((q.x + q.y) * q.z);
  }
  float vnoise(vec2 p) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    f = f * f * (3.0 - 2.0 * f);
    return mix(mix(hash(i), hash(i + vec2(1.0, 0.0)), f.x),
               mix(hash(i + vec2(0.0, 1.0)), hash(i + vec2(1.0, 1.0)), f.x), f.y);
  }
`;

const GROUND_VERT = /* glsl */ `
  varying vec3 vWorld;
  ${SHARED}
  void main() {
    vec3 p = position;
    p.y = groundY(p.xz);
    vWorld = p;
    gl_Position = projectionMatrix * viewMatrix * vec4(p, 1.0);
  }
`;

// Opaque ground under the grains: holds the horizon, the soft shadow under
// the sculpture and the accent light the meter spills onto the sand.
const GROUND_FRAG = /* glsl */ `
  precision highp float;
  uniform vec3 uSpill;
  varying vec3 vWorld;
  ${SHARED}
  void main() {
    float r = length(vWorld.xz);
    float contact = smoothstep(0.8, 2.4, footprint(vWorld.xz));
    float pool = smoothstep(8.0, 0.8, r);
    vec3 col = vec3(0.0042) * pool * mix(0.3, 1.0, contact);
    col += uSpill * 0.006 * exp(-r * r / 0.7);
    gl_FragColor = vec4(col, 1.0);
  }
`;

const GRAIN_VERT = /* glsl */ `
  attribute float aSeed;
  uniform float uTime, uDpr, uFocus, uPxPerRad, uIntro, uPool;
  varying float vLight;
  varying float vSpill;
  ${SHARED}
  ${NOISE}
  void main() {
    vec3 p = position;
    float r = length(p.xz);

    float dune = vnoise(p.xz * 1.5) * 0.05 + vnoise(p.xz * 5.2) * 0.01;
    p.y = groundY(p.xz) + dune + 0.006;

    vec4 mv = modelViewMatrix * vec4(p, 1.0);
    float dist = max(-mv.z, 0.05);

    // Focus is per grain: out-of-focus grains swell and dim, which reads as
    // depth of field without a depth pass.
    float blur = clamp(abs(dist - uFocus) / dist * 0.9, 0.0, 1.0);
    float world = mix(0.005, 0.014, pow(fract(aSeed * 13.7), 9.0));
    float size = world * uPxPerRad / dist;
    float floorPx = 1.0 * uDpr;
    float grown = max(size, floorPx) + blur * blur * 5.0 * uDpr;
    gl_PointSize = min(grown, 12.0 * uDpr);

    float energy = min(1.0, (size * size) / (floorPx * floorPx) + 0.35) / (1.0 + blur * blur * 6.0);
    float pool = smoothstep(uPool, 0.8, r);
    float near = smoothstep(1.2, 5.0, dist);
    float contact = mix(0.35, 1.0, smoothstep(0.8, 2.6, footprint(p.xz)));
    float rake = 0.4 + 0.6 * smoothstep(0.012, 0.045, dune + 0.016 * sin(p.x * 3.0 + p.z * 2.0));
    float spark = mix(0.12, 1.0, pow(fract(aSeed * 91.7), 5.0));
    float twinkle = 0.7 + 0.3 * sin(uTime * (0.5 + aSeed * 2.0) + aSeed * 60.0);

    vLight = pool * near * contact * rake * spark * twinkle * energy;
    vLight *= 1.0 - uIntro;
    // The meter lights the sand right under the sculpture.
    vSpill = exp(-r * r / 0.9);

    gl_Position = projectionMatrix * mv;
  }
`;

const GRAIN_FRAG = /* glsl */ `
  precision highp float;
  uniform vec3 uSpill;
  varying float vLight;
  varying float vSpill;
  void main() {
    vec2 q = gl_PointCoord - 0.5;
    float a = smoothstep(0.5, 0.12, length(q));
    vec3 col = (vec3(vLight) + uSpill * vSpill * 0.45) * a * 0.42;
    gl_FragColor = vec4(col, 1.0);
  }
`;

const SKY_FRAG = /* glsl */ `
  precision highp float;
  varying vec3 vDir;
  void main() {
    vec3 d = normalize(vDir);
    float band = exp(-max(d.y + 0.02, 0.0) * 3.0);
    vec3 col = vec3(0.0065) * band;
    gl_FragColor = vec4(col, 1.0);
  }
`;

export class Floor {
  constructor(count, accent) {
    this.group = new THREE.Group();
    this.uniforms = {
      uTime: { value: 0 },
      uDpr: { value: 1 },
      uFocus: { value: 5 },
      uPxPerRad: { value: 1000 },
      uIntro: { value: 1 },
      uBend: { value: 0.03 },
      uFoot: { value: new THREE.Vector2(0.5, 0.2) },
      uSpill: { value: accent },
      uPool: { value: 8.5 },
    };

    const disc = new THREE.RingGeometry(0.0005, REACH + 2, 256, 160).rotateX(-Math.PI / 2);
    const ground = new THREE.Mesh(
      disc,
      new THREE.ShaderMaterial({ uniforms: this.uniforms, vertexShader: GROUND_VERT, fragmentShader: GROUND_FRAG }),
    );
    ground.frustumCulled = false;
    this.group.add(ground);

    const positions = new Float32Array(count * 3);
    const seeds = new Float32Array(count);
    for (let i = 0; i < count; i++) {
      const r = REACH * Math.random() ** 0.72;
      const a = Math.random() * Math.PI * 2;
      positions[i * 3] = Math.cos(a) * r;
      positions[i * 3 + 2] = Math.sin(a) * r;
      seeds[i] = Math.random();
    }
    const geometry = new THREE.BufferGeometry();
    geometry.setAttribute('position', new THREE.BufferAttribute(positions, 3));
    geometry.setAttribute('aSeed', new THREE.BufferAttribute(seeds, 1));

    this.grains = new THREE.Points(
      geometry,
      new THREE.ShaderMaterial({
        uniforms: this.uniforms,
        vertexShader: GRAIN_VERT,
        fragmentShader: GRAIN_FRAG,
        transparent: true,
        depthWrite: false,
        blending: THREE.AdditiveBlending,
      }),
    );
    this.grains.frustumCulled = false;
    this.group.add(this.grains);

    const sky = new THREE.Mesh(
      new THREE.SphereGeometry(60, 32, 24),
      new THREE.ShaderMaterial({
        side: THREE.BackSide,
        depthWrite: false,
        vertexShader: `varying vec3 vDir; void main(){ vDir = position; gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0); }`,
        fragmentShader: SKY_FRAG,
      }),
    );
    sky.renderOrder = -1;
    this.group.add(sky);
  }
}
