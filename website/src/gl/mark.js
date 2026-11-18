import * as THREE from 'three';
import { OUTLINE, BARS, BOUNDS, ACCENT } from './logo.js';

// One SVG unit in world units. The bubble is 49 units tall, so the sculpture
// is 1.0 tall and its top sits at 1.18, the height the camera path was keyed to.
const UNIT = 0.0204;
const HOVER = 0.18;
const DEPTH = 0.26;
const BEVEL = 0.028;
const RECESS = 0.05;
const FACE_Z = DEPTH / 2 + BEVEL;

export const FOOTPRINT = [((BOUNDS.maxX - BOUNDS.minX) / 2) * UNIT, FACE_Z + 0.04];

// SVG space (y down, origin top left) to world space (y up, centred on x,
// tail tip resting HOVER above the sand).
const toWorld = (x, y) => new THREE.Vector2((x - 32) * UNIT, (BOUNDS.maxY - y) * UNIT + HOVER);

function stadium([x, y, w, h], inset = 0) {
  const r = w / 2 - inset;
  const cx = x + w / 2;
  const top = y + w / 2;
  const bottom = y + h - w / 2;
  const pts = [];
  for (let i = 0; i <= 24; i++) {
    const a = Math.PI + (i / 24) * Math.PI;
    pts.push(toWorld(cx + Math.cos(a) * r, top + Math.sin(a) * r));
  }
  for (let i = 0; i <= 24; i++) {
    const a = (i / 24) * Math.PI;
    pts.push(toWorld(cx + Math.cos(a) * r, bottom + Math.sin(a) * r));
  }
  return pts;
}

const STONE = { color: 0x0a0a0c, metalness: 0, clearcoat: 0.7, clearcoatRoughness: 0.1 };

// The mark's 64-unit box in world space, for placing the HTML wordmark beside
// it: an 88 px mark, a 16 px gap and 76 px text.
export const LOCKUP = {
  right: (64 - 32) * UNIT,
  centreY: (BOUNDS.maxY - 32) * UNIT + HOVER,
  half: 32 * UNIT,
  z: FACE_Z,
  // Middle of the side-by-side lockup, which the opening shot is framed on.
  lockupCentre: 2.03,
};

// ExtrudeGeometry puts both faces in one group and shades every triangle flat.
// Rebuild it as front / back / walls: faces stay flat, walls are smoothed.
function regroup(source) {
  const pos = source.attributes.position.array;
  const [caps, walls] = source.groups;
  const buckets = [[], [], []];
  const a = new THREE.Vector3();
  const b = new THREE.Vector3();
  const c = new THREE.Vector3();

  const faceNormal = (i) => {
    a.fromArray(pos, i * 3);
    b.fromArray(pos, i * 3 + 3);
    c.fromArray(pos, i * 3 + 6);
    return new THREE.Vector3().subVectors(c, b).cross(a.clone().sub(b)).normalize();
  };

  for (let i = caps.start; i < caps.start + caps.count; i += 3) {
    buckets[faceNormal(i).z > 0 ? 0 : 1].push(i);
  }
  for (let i = walls.start; i < walls.start + walls.count; i += 3) buckets[2].push(i);

  const key = (i) => `${Math.round(pos[i * 3] * 1e5)},${Math.round(pos[i * 3 + 1] * 1e5)},${Math.round(pos[i * 3 + 2] * 1e5)}`;
  const smooth = new Map();
  for (const i of buckets[2]) {
    const fn = faceNormal(i).multiplyScalar(a.clone().sub(b).cross(c.clone().sub(b)).length());
    for (let k = 0; k < 3; k++) {
      const id = key(i + k);
      if (!smooth.has(id)) smooth.set(id, new THREE.Vector3());
      smooth.get(id).add(fn);
    }
  }

  const outPos = [];
  const outNormal = [];
  const out = new THREE.BufferGeometry();
  let start = 0;

  buckets.forEach((tris, group) => {
    for (const i of tris) {
      for (let k = 0; k < 3; k++) {
        const j = i + k;
        outPos.push(pos[j * 3], pos[j * 3 + 1], pos[j * 3 + 2]);
        if (group === 2) {
          const s = smooth.get(key(j)).clone().normalize();
          outNormal.push(s.x, s.y, s.z);
        } else {
          outNormal.push(0, 0, group === 0 ? 1 : -1);
        }
      }
    }
    out.addGroup(start, tris.length * 3, group);
    start += tris.length * 3;
  });

  out.setAttribute('position', new THREE.Float32BufferAttribute(outPos, 3));
  out.setAttribute('normal', new THREE.Float32BufferAttribute(outNormal, 3));
  source.dispose();
  return out;
}

// The mark as a slab of dark polished stone. The meter is cut through it and
// lit from inside with the accent colour.
export class Mark {
  constructor() {
    this.group = new THREE.Group();
    this.accent = new THREE.Color(ACCENT);

    const outline = new THREE.Shape(OUTLINE.map((p) => toWorld(p.x, p.y)));
    outline.holes = BARS.map((bar) => new THREE.Path(stadium(bar)));
    const solid = new THREE.ExtrudeGeometry(outline, {
      depth: DEPTH,
      bevelEnabled: true,
      bevelThickness: BEVEL,
      bevelSize: BEVEL,
      bevelOffset: -BEVEL,
      bevelSegments: 10,
      curveSegments: 1,
    });
    solid.translate(0, 0, -DEPTH / 2);

    const materials = [0.32, 0.36, 0.28].map((roughness) => new THREE.MeshPhysicalMaterial({ ...STONE, roughness }));
    this.body = new THREE.Mesh(regroup(solid), materials);
    this.group.add(this.body);

    // The meter glows from inside the bars, set back from both faces. The
    // colour is kept low enough that tone mapping preserves the accent's hue;
    // bloom supplies the glow.
    this.barMaterial = new THREE.MeshBasicMaterial({ color: this.accent.clone().multiplyScalar(1.35) });
    const inner = DEPTH + 2 * BEVEL - 2 * RECESS;
    for (const spec of BARS) {
      const bar = new THREE.ExtrudeGeometry(new THREE.Shape(stadium(spec, 0.15)), { depth: inner, bevelEnabled: false });
      bar.translate(0, 0, -inner / 2);
      this.group.add(new THREE.Mesh(bar, this.barMaterial));

      const [x, y, w, h] = spec;
      const centre = toWorld(x + w / 2, y + h / 2);
      const light = new THREE.PointLight(this.accent, 0.06, 0.45, 2);
      light.position.set(centre.x, centre.y, 0);
      this.group.add(light);
    }
  }

  update(time) {
    this.group.position.y = Math.sin(time * 0.7) * 0.014;
    this.group.rotation.y = Math.sin(time * 0.23) * 0.045;
  }
}
