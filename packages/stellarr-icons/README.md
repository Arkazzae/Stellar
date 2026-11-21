# `@stellarr/icons`

Stellarr's vector icon package, shared by the web client (`web/`), Cockpit (`web/src/cockpit/`), and [`@stellarr/ui`](../stellarr-ui/README.md). The same glyph definitions generate the standalone SVG files in [`branding/icons/`](../../branding/icons/).

The package is private and consumed directly as TypeScript source (`exports` points to `src/index.ts`; Vite and Vitest compile it as part of the importing application). React is its only runtime dependency.

## Exports

- Components and context: `Icon`, `IconVariantProvider`, `DEFAULT_ICON_VARIANT`
- Generated glyph tables: `ICON_GLYPHS`, `ICON_VARIANTS`, `iconPaths`
- Types: `IconName`, `IconVariant`, `IconProps`, `IconGlyph`, `IconPath`

Every icon is drawn from a single geometric source in two variants:

- **`broken`** (default): each primary contour is opened by a single `3.1 px` gap.
- **`linear`**: closed, continuous contours.

Pass `variant="linear"` or `variant="broken"` on an individual `<Icon>` or wrap a subtree in `<IconVariantProvider variant="...">`.

## Source files and code generation

- [`src/source/glyphs.ts`](src/source/glyphs.ts): Declarative glyph definitions. Plain SVG path strings render identically in both variants; `brk(d, [x, y])` introduces a contour gap at `[x, y]` in the `broken` variant.
- [`src/source/geometry.ts`](src/source/geometry.ts): Shared geometric primitives (`rect`, `circle`, `arc`, `poly`), transforms, and contour knockouts/gaps.
- [`src/paths.ts`](src/paths.ts): Generated static path table imported at runtime. Do not edit by hand.

After modifying [`src/source/glyphs.ts`](src/source/glyphs.ts) or [`src/source/geometry.ts`](src/source/geometry.ts), regenerate [`src/paths.ts`](src/paths.ts) and [`branding/icons/`](../../branding/icons/) from the repository root:

```sh
npm run icons:build -w @stellarr/icons              # regenerate src/paths.ts and branding/icons/
npm run icons:build -w @stellarr/icons -- --check   # verify generated files match source (run in CI)
npm run icons:build -w @stellarr/icons -- --audit   # check for stroke crowding and clearance violations
```

## Drawing specifications

- **Canvas and stroke**: `24 × 24` `viewBox`, `1.5 px` stroke width, round caps, round joins, `currentColor`.
- **Live area**: `2..22`. Rectangular containers sit at `2.5..21.5` with soft corner radii (up to `6` at full size); full circles extend to `2..22` for optical balance.
- **Stroke clearance**: Separate strokes either join or maintain at least `2.5` units between centerlines (~`1 px` clear space at `24 px`), enforced by `--audit`.
- **Broken variant gaps**: One gap of length `GAP` (`3.1`) per primary contour on its longest straight segment (preferring the upper-right quadrant where geometry permits). Dots, short ticks, and small rings remain closed in both variants.
- **Diagonal slash knockouts**: Mute/disable slashes knock out underlying contours within `2.5` units in both variants (`knockLine`, `knockNear`).
- **Accessibility**: `<Icon>` renders `aria-hidden="true"` by default; place accessible labels on the enclosing interactive control.

Brand logos, mascot artwork, default avatars, and audio waveforms live under [`branding/`](../../branding/README.md).

## Verification

From the repository root:

```bash
npm run typecheck:icons
npm run test:icons
npm run icons:build -w @stellarr/icons -- --check
```
