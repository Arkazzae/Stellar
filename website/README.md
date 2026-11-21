# Stellarr project website

Source for the public Stellarr website (`https://stellarr.online`): a three-section single-page site backed by an interactive WebGL 2 scene of the Stellarr mark. Scroll position, arrow keys, and the right-hand section rail drive the camera along a continuous 3D path.

Built with vanilla JavaScript, [Vite](https://vite.dev), and [three.js](https://threejs.org). `website/` is a standalone npm project outside the root monorepo workspaces and is not included in the Stellarr container image.

## Local development

```bash
cd website
npm ci
npm run dev       # start Vite dev server with hot reload
npm run build     # build static site into dist/
npm run preview   # preview dist/ locally
```

The 3D background scene requires WebGL 2. When WebGL 2 is unavailable, the page displays a fallback notice while keeping all content sections scrollable.

## Project structure

| Area | Path |
| --- | --- |
| Page copy, links, and install commands | `index.html` |
| Section definitions, scroll lengths, and camera keyframes (`SECTIONS`, `CAMERA`) | `src/config.js` |
| Colors, typography, and layout rules | `src/styles.css` |
| Three.js scene (3D mark geometry, particle sand, lighting, camera rig) | `src/gl/` |
| Scroll controller, section rail, text reveal animations, wordmark | `src/ui/` |
| Application entry point wiring UI and WebGL | `src/main.js` |

## Shared branding assets

The build reads brand assets directly from [`../branding/`](../branding/README.md) rather than duplicating files:

- The 3D extruded mark is constructed from `branding/logo/mark.svg` at build time.
- Web fonts are loaded from `branding/guidelines/fonts/`.
- The header mark, astronaut mascot, and linear UI icons in `index.html` are inlined from `branding/logo/small.svg`, `branding/mascot/expressions/happy.svg`, and `branding/icons/linear/`.
- The Vite build copies the root [`../install.sh`](../install.sh) script into `dist/install.sh` so `https://stellarr.online/install.sh` always serves the current installer as `text/plain`.

Because the build reads files from `../branding/` and `../install.sh`, always build from a full repository checkout.

## Deploying to Cloudflare Pages

The site deploys to the `stellarr` Cloudflare Pages project (`main` is the production branch):

```bash
cd website
npm ci
npm run build
npx wrangler pages deploy dist --project-name stellarr --branch main
```

Passing a `--branch` other than `main` publishes a preview deployment URL.

`public/_headers` configures immutable 1-year caching for content-hashed bundles under `/assets/*` and immediate revalidation for `index.html` and `install.sh` so new deployments take effect immediately.
