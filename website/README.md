# Stellarr project website

Source for the public Stellarr website (`https://stellarr.online`): a single scrolling page that shows what Stellarr does and how to install it. The call settings, translation example, and theme studio are small working copies of the app's own controls, with the presets, hints, palettes, and labels taken from the web client.

Built with vanilla JavaScript and [Vite](https://vite.dev), with no runtime dependencies. `website/` is a standalone npm project outside the root monorepo workspaces and is not included in the Stellarr container image.

## Local development

```bash
cd website
npm ci
npm run dev       # start Vite dev server with hot reload
npm run build     # build static site into dist/
npm run preview   # preview dist/ locally
```

## Project structure

| Area | Path |
| --- | --- |
| Page copy, links, install command and the icon sprite | `index.html` |
| Colors, typography and layout | `src/styles.css` |
| Star field, pause button, call settings, translation example, theme studio, copy button | `src/main.js` |
| Resized images used by the page | `src/images/` |

The star field behind the hero and the hosting section is drawn on a canvas and leans with the pointer. It stops when the section is off screen, when the tab is hidden, when the visitor presses the pause button in the hero (remembered in `localStorage`), and under `prefers-reduced-motion`, where the hero also shows a still frame instead of the tour film.

## Shared branding assets

The build reads these files directly from [`../branding/`](../branding/README.md) rather than duplicating them:

- The header, drawer and footer logos and the favicon: `branding/logo/lockup-small.svg`, `lockup.svg` and `small.svg`.
- The web fonts: `branding/guidelines/fonts/`.
- The tour film in the hero: `branding/film/stellarr.webp`.

The icons in the sprite in `index.html` are copied from `branding/icons/linear/`; copy the new path from there when an icon changes.

The images in `src/images/` are sized and exported as WebP for the site.

| File | Source |
| --- | --- |
| `film/app.webp`, `film/screenshare.webp`, `film/cockpit.webp` | Frames 100, 410 and 590 of `branding/film/stellarr.webp` |
| `scenes/10-reading-nook.webp`, `scenes/13-observatory.webp` | `branding/backgrounds/`, resized to 1400 px wide |
| `scenes/group-chat-lounge*.webp` | ImageGen scene based on Reading Nook and Zero-G Game Night, with a tighter mobile crop |
| `scenes/server-launch*.webp` | ImageGen panorama based on Launch Mission, Home Server Workshop and Observatory, with a tighter mobile crop |
| `avatars/*.webp` | `branding/avatars/`, resized to 96 × 96 |
| `mascot/*.webp` | `branding/mascot/illustrations/`, trimmed to the drawing and resized to 520 px wide |

## Deploying

`stellarr.online` is the Cloudflare Pages project `stellarr`. After a push to `main` that changes the site, `branding/logo`, `branding/film`, the fonts or `install.sh`, the `website-deploy` job in [`ci.yml`](../.github/workflows/ci.yml) builds `dist/` and publishes it with `wrangler pages deploy`, using the `CLOUDFLARE_API_TOKEN` and `CLOUDFLARE_ACCOUNT_ID` repository secrets. Pull requests never deploy.

To publish by hand from a full checkout:

```bash
cd website
npm ci
npm run build
npx wrangler pages deploy dist --project-name stellarr --branch main
```

The Vite build copies the root [`../install.sh`](../install.sh) script into `dist/install.sh` so `https://stellarr.online/install.sh` always serves the current installer as `text/plain`.

Because the build reads files from `../branding/` and `../install.sh`, always build from a full repository checkout.
