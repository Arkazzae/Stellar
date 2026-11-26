# Stellarr Project Website (`website/`)

Source for `https://stellarr.online`—a zero-dependency vanilla JavaScript and [Vite](https://vite.dev) landing page featuring interactive previews of Stellarr's voice DSP controls, offline chat translation, and theme studio, plus the `/install.sh` and `/server-downloads/*` endpoints.

`website/` is a standalone npm project outside the root workspaces and is excluded from the Stellarr container image. Always build from a full repository checkout because the build also reads the Cockpit fonts and `../install.sh`.

## Local development

```bash
cd website
npm ci
npm run dev       # Vite dev server with hot reload
npm run build     # emit static site + /install.sh into dist/
npm run preview   # preview production build locally
```

## Directory layout and shared assets

| Path | Role |
| --- | --- |
| `index.html` | Landing page copy, install snippet, interactive demos, and inline SVG icon sprite |
| `src/styles.css`, `src/main.js` | Layout/tokens and interactive logic (starfield canvas pausing off-screen, in background tabs, on user toggle, or under `prefers-reduced-motion`; voice, translation, and theme demos) |
| `src/images/` | WebP scenes, avatars (`96×96`), mascot cutouts (`520 px`), and still frames (`100`, `410`, `590`) from `src/images/film/stellarr.webp` |
| `functions/server-downloads/` | Cloudflare Pages Function resolving stable release channels and serving installer bundles from public GHCR OCI artifacts |
| `src/images/logo/`, `src/images/film/stellarr.webp` | Logos and the published tour shared with the documentation and project README |
| `../web/src/cockpit/assets/fonts/`, `../install.sh` | Typefaces and the installer served at `/install.sh` |

## Deployment (`stellarr.online`)

Pushes to `main` touching `website/`, the shared fonts, or `install.sh` trigger the `website-deploy` job in [`.github/workflows/ci.yml`](../.github/workflows/ci.yml) (`wrangler pages deploy` to Cloudflare Pages project `stellarr`).

Manual deployment from a full checkout:

```bash
cd website && npm ci && npm run build
npx wrangler pages deploy dist --project-name stellarr --branch main
```
