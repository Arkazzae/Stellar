# Stellarr UI icons

Exported SVG files for Stellarr's UI icon family in two variants:

- [`broken/`](broken/): default variant, each main contour opened by one gap
- [`linear/`](linear/): closed continuous contours

Each file uses a 24 × 24 `viewBox`, a 1.5 px stroke, round caps and joins, and `currentColor`. Filenames convert icon keys to kebab-case (`micSlash` → `mic-slash.svg`).

These SVGs are generated from [`packages/stellarr-icons/src/source/glyphs.ts`](../../packages/stellarr-icons/src/source/glyphs.ts). See [`packages/stellarr-icons/README.md`](../../packages/stellarr-icons/README.md) for drawing rules and the `@stellarr/icons` React component.

```bash
npm run icons:build -w @stellarr/icons            # regenerate SVGs and path table
npm run icons:build -w @stellarr/icons -- --check # verify outputs match source
```
