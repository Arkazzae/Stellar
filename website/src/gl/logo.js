import { SVGLoader } from 'three/addons/loaders/SVGLoader.js';
import markSvg from '../../../branding/logo/mark.svg?raw';

// The sculpture is built from the logo file itself, so it follows the logo if
// the logo is redrawn. Units are the 64 x 64 viewBox of the file, y down.
const doc = new DOMParser().parseFromString(markSvg, 'image/svg+xml');

// The bubble outline, tail included.
const bubble = new SVGLoader().parse(markSvg).paths[0].toShapes(true)[0];
export const OUTLINE = bubble.extractPoints(40).shape;

// The meter bars as [x, y, width, height]; each has fully rounded ends.
export const BARS = [...doc.querySelectorAll('rect')].map((r) => ['x', 'y', 'width', 'height'].map((a) => Number(r.getAttribute(a))));

export const ACCENT = doc.querySelector('g').getAttribute('fill');

// Extent of the outline, rounded outward to whole units.
export const BOUNDS = {
  minX: Math.floor(Math.min(...OUTLINE.map((p) => p.x))),
  maxX: Math.ceil(Math.max(...OUTLINE.map((p) => p.x))),
  minY: Math.floor(Math.min(...OUTLINE.map((p) => p.y))),
  maxY: Math.ceil(Math.max(...OUTLINE.map((p) => p.y))),
};
