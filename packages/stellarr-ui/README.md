# `@stellarr/ui`

Shared UI component library and overlay primitives used by the Stellarr web client (`web/`) and Cockpit (`web/src/cockpit/`). The package is private and exposes three entry points:

- `@stellarr/ui`: React UI components and the `cx` class-name helper.
- `@stellarr/ui/styles.css`: CSS layer stylesheet (`@layer components.ui`).
- `@stellarr/ui/behavior`: Low-level overlay, layer stack, scroll lock, focus trap, and positioning primitives (see [Behavior primitives](#behavior-primitives)).

The package is consumed directly as TypeScript source (`exports` points to `src/index.ts`; Vite and Vitest compile it as part of the importing application). Inside the package, modules use relative imports (`@/` path aliases belong to the consuming apps). Its only dependencies are React and [`@stellarr/icons`](../stellarr-icons/README.md).

## Components

| Export | Description |
| --- | --- |
| `Button` | Action and toggle button. `variant`: `primary` (accent fill, default), `secondary` (outlined), `quiet` (borderless), `danger` (danger outline), `danger-solid` (danger fill). `size`: `default`, `sm`, `icon`, `icon-sm`. Passing `aria-pressed` to a `secondary` button styles it as a stateful toggle. |
| `Badge` | Status and metadata pill. `variant`: `secondary`, `outline`, `success`, `warning`, `destructive`. |
| `Card`, `CardHeader`, `CardTitle`, `CardDescription`, `CardContent` | Elevated container surface with structured header and body slots. |
| `Alert`, `AlertTitle`, `AlertDescription` | Inline status callout (`variant="destructive"` for errors). |
| `Input`, `Label` | Text input control and form label. Setting `aria-invalid` applies error border styling. |
| `Switch` | Accessible boolean toggle (`role="switch"`), controlled via `checked` and `onCheckedChange`. |
| `Segmented` | Mutually exclusive radio group with roving tabindex (single tab stop, Arrow keys, Home/End). Controlled via `options`, `value`, and `onChange`. `variant`: `cards` (bordered card grid, default) or `pill` (compact segmented track). Each option supports `label`, `detail`, `leading`, `title`, and `tone`. |
| `Tabs`, `TabsList`, `TabsTrigger`, `TabsContent` | Accessible tab bar with roving tabindex and arrow-key activation. Only the active panel is mounted. Pass `unstyled` for custom tab chrome. |
| `Table`, `TableHeader`, `TableBody`, `TableRow`, `TableHead`, `TableCell` | Data table with sticky header support inside scrollable containers. |
| `Dialog` + `DialogContent`, `DialogHeader`, `DialogTitle`, `DialogDescription`, `DialogFooter`, `DialogClose`, `DialogTrigger` | Centered modal dialog with focus trapping, backdrop scrim, and scroll lock. |
| `Sheet` + `SheetContent` (`side`: `right`, `left`), `SheetHeader`, `SheetTitle`, `SheetDescription`, `SheetClose`, `SheetTrigger` | Edge-anchored modal drawer. |
| `Dropdown` | Single-select combobox with listbox popup (single tab stop, Arrow keys, Home/End, and typeahead search). Controlled via `options`, `value`, and `onChange`; labeled via `label` or `id` (`<label for>`). `variant`: `field` (full-width form control, default) or `quiet` (compact inline control). Options support `label`, `detail`, `icon` (`IconName` from `@stellarr/icons`), `lang`, and `disabled`. Supports `placeholder`, `name` (hidden form input), `invalid`, and `aria-describedby`. |
| `DropdownMenu` + `DropdownMenuTrigger`, `DropdownMenuContent`, `DropdownMenuItem`, `DropdownMenuLabel`, `DropdownMenuSeparator` | Action menu anchored to a trigger button. |
| `Tooltip` + `TooltipTrigger`, `TooltipContent` | Floating label shown on pointer hover (after `delayDuration`, default `300 ms`) and immediately on keyboard focus. |
| `Spinner` | Animated loading indicator. |

Shared conventions across all components:

- **Utility class override**: Every component accepts `className`. Because package styles live in `@layer components.ui`, Tailwind utility classes in `className` always take precedence regardless of selector specificity.
- **Polymorphic rendering (`asChild`)**: `Button`, overlay triggers, `DialogClose`, and `DropdownMenuItem` support `asChild` via `Slot` to merge behavior and styling onto a single child element (such as `<a href>` or `<Link>`).
- **Locale-agnostic markup**: No user-visible strings are hardcoded. `DialogContent` and `SheetContent` require an explicit `closeLabel` prop for the close button's accessible name.
- **Coordinated modal stack**: Dialogs, sheets, and menus hide background content from assistive technology, lock background scrolling, trap `Tab`, close only the topmost active layer on `Escape`, and restore focus to the trigger on close.
- **Viewport-aware floating placement**: Menus, tooltips, and dropdown lists position themselves relative to their trigger, flip across axes when constrained by viewport edges, and clamp within the window. Coordinates are exposed as private `--_x` and `--_y` custom properties rather than inline `left`/`top` styles so responsive media queries can override positioning.

## Styles and CSS architecture

Import `@stellarr/ui/styles.css` immediately after Tailwind CSS:

```css
@import "tailwindcss";
@import "@stellarr/ui/styles.css";
```

Because `components.ui` is nested inside Tailwind's `components` layer, any utility class passed via `className` (for example, `<Card className="gap-0 py-0">`) or any rule declared in the app's own `@layer components` overrides the default component rules without fighting specificity. Component classes use the `ui-` prefix, and states/variants use `data-variant`, `data-size`, `data-side`, and `data-state` attributes.

Consuming applications never write CSS selectors targeting `.ui-*` classes directly. Layout adjustments are passed via `className`, and visual customization is done through CSS custom properties (`--ui-*` tokens) scoped to a container or `data-*` attribute:

```css
.confirm-footer {
  --ui-button-secondary-bg: var(--surface-overlay);
}

.permission-pill [data-tone="deny"] {
  --ui-segmented-item-on-bg: var(--danger-soft);
}
```

### Design tokens

Visual styling is driven by two tiers of CSS custom properties. `@stellarr/ui/styles.css` consumes these variables without defining global values.

#### 1. System tokens (required)

Consuming applications must define every system token (see [`web/src/cockpit/index.css`](../../web/src/cockpit/index.css) for Cockpit and [`web/src/stellarr/styles/ui-tokens.css`](../../web/src/stellarr/styles/ui-tokens.css) for the web client):

| Token | Used for |
| --- | --- |
| `--ui-unit` | Base sizing unit (`1px` in the web client; `calc(1rem / 16)` in Cockpit for fluid root scaling). All component dimensions multiply `--ui-unit`, while hairlines (`1px`) and focus ring widths (`2px`–`4px`) stay in physical `px` |
| `--ui-page` | Base page background (sheets and tooltip text contrast) |
| `--ui-surface` | Cards, dialogs, active tab trigger, and sticky table headers |
| `--ui-surface-muted` | Neutral alerts and `Segmented` `pill` track background |
| `--ui-popover` | Dropdown menus and hovered/active `pill` segments |
| `--ui-hover` | Hovered `quiet` buttons, highlighted menu items, `secondary` badges, and unchecked `Switch` track |
| `--ui-text`, `--ui-text-muted`, `--ui-text-faint` | Primary text, secondary text (descriptions, table headers, placeholders, idle card labels), and faint text (unchecked switch thumb, idle `quiet` button, card detail line, idle `pill` label) |
| `--ui-line` | Default borders and dividers |
| `--ui-line-subtle` | Idle `Segmented` cards and `pill` track border |
| `--ui-line-field` | Text input border |
| `--ui-line-strong` | Hovered `secondary` button and hovered `Segmented` card border |
| `--ui-ring`, `--ui-ring-offset` | Focus ring color and inner offset background |
| `--ui-accent`, `--ui-accent-hover`, `--ui-on-accent` | `primary` button fill/hover, checked `Switch` track, checked `Segmented` card border/tint, and text selection |
| `--ui-danger`, `--ui-on-danger` | `danger-solid` button fill/foreground and `danger` button border |
| `--ui-danger-text`, `--ui-danger-soft` | `danger` button label, destructive menu items/badges/alerts, and invalid input borders |
| `--ui-success-text`, `--ui-success-soft`, `--ui-warning-text`, `--ui-warning-soft` | Semantic `success` and `warning` badges |
| `--ui-scrim` | Modal backdrop overlay behind dialogs and sheets |
| `--ui-elevation-low`, `--ui-elevation-menu`, `--ui-elevation-modal` | Box shadows for tooltips, menus/listboxes, and modal dialogs/sheets |
| `--ui-radius-xs`, `--ui-radius-sm`, `--ui-radius-md`, `--ui-radius-lg`, `--ui-radius-xl` | Corner radii across close buttons, menu items, icon buttons, inputs, menus, alerts, cards, and dialogs |
| `--ui-ease` | Transition timing function |
| `--ui-dur-fast`, `--ui-dur-base` | Transition durations for color/opacity changes and `Switch` thumb movement |

#### 2. Component tokens (optional overrides)

Component tokens follow `--ui-<component>-<part>-<property>` and fall back to the web client's default styling when unset. Cockpit overrides select component tokens in [`web/src/cockpit/index.css`](../../web/src/cockpit/index.css).

`Switch`:

| Token | Fallback | Used for |
| --- | --- | --- |
| `--ui-switch-w`, `--ui-switch-h` | `44` and `26` units | Track width and height |
| `--ui-switch-bg` | `--ui-hover` | Track background when unchecked |
| `--ui-switch-border` | `--ui-line` | Track border when unchecked |
| `--ui-switch-on-bg` | `--ui-accent` | Track background when checked |
| `--ui-switch-on-border` | `--ui-accent` | Track border when checked |
| `--ui-switch-thumb-size` | `20` units | Thumb diameter |
| `--ui-switch-thumb-bg` | `--ui-text-faint` | Thumb color when unchecked |
| `--ui-switch-thumb-on-bg` | `--ui-on-accent` | Thumb color when checked |
| `--ui-switch-thumb-align` | `flex-start` | Track `align-items` (`center` centers the thumb vertically) |
| `--ui-switch-thumb-y` | `2` units | Thumb top offset (`0px` when centered) |
| `--ui-switch-thumb-shadow` | `none` | Thumb box shadow |
| `--ui-switch-thumb-x` | `2` units | Thumb horizontal offset when unchecked |
| `--ui-switch-thumb-on-x` | `20` units | Thumb horizontal offset when checked |
| `--ui-switch-disabled-opacity` | `1` | Opacity when disabled |
| `--ui-switch-hit-inset` | `0` | Touch target expansion (`inset` on the hit pseudo-element) |

`Button`:

| Token | Fallback | Used for |
| --- | --- | --- |
| `--ui-button-min-height` | `auto` | Minimum button height (increased on coarse pointers) |
| `--ui-button-radius` | pill (`calc(infinity * 1px)`) | Corner radius for labeled buttons |
| `--ui-button-gap` | `normal` | Gap between icon and label |
| `--ui-button-sm-gap` | `--ui-button-gap` | Gap between icon and label in `sm` size |
| `--ui-button-icon-size` | `auto` | Explicit icon width/height override |
| `--ui-button-white-space` | `inherit` | Label wrapping (`white-space`) |
| `--ui-button-font-size` | `13` units | Label font size |
| `--ui-button-line-height` | `inherit` | Label line height |
| `--ui-button-font-weight` | `600` | Label font weight |
| `--ui-button-padding-x` | `18` units | Horizontal padding for default size |
| `--ui-button-sm-font-size`, `--ui-button-sm-line-height` | Inherits default size tokens | Label font size and line height for `sm` size |
| `--ui-button-sm-padding-x` | `14` units | Horizontal padding for `sm` size |
| `--ui-button-icon-only-radius` | `--ui-radius-sm` | Corner radius for `icon` and `icon-sm` sizes |
| `--ui-button-icon-only-min-width` | `auto` | Minimum width for `icon` and `icon-sm` sizes |
| `--ui-button-disabled-opacity` | `1` (`0.4` for `secondary`) | Opacity when disabled |
| `--ui-button-primary-bg` | `--ui-accent` | Idle background fill for `primary` variant |
| `--ui-button-secondary-border` | `--ui-line` | Idle border color for `secondary` variant |
| `--ui-button-secondary-bg` | `transparent` | Idle background fill for `secondary` variant |
| `--ui-button-secondary-text` | `--ui-text-muted` | Idle text color for `secondary` variant |
| `--ui-button-secondary-hover-border` | `--ui-line-strong` | Hover border color for `secondary` variant |
| `--ui-button-secondary-hover-bg` | `--ui-button-secondary-bg` | Hover background fill for `secondary` variant |
| `--ui-button-secondary-hover-text` | `--ui-text` | Hover text color for `secondary` variant |
| `--ui-button-secondary-pressed-border`, `--ui-button-secondary-pressed-bg` | Inherits hover values | Border and background for active toggle (`aria-pressed="true"`) |
| `--ui-button-secondary-pressed-text` | `--ui-text` | Text color for active toggle (`aria-pressed="true"`) |
| `--ui-button-secondary-unpressed-text` | `--ui-button-secondary-text` | Text color for inactive toggle (`aria-pressed="false"`) |
| `--ui-button-quiet-text` | `--ui-text-faint` | Idle text color for `quiet` variant |
| `--ui-button-quiet-hover-bg` | `--ui-hover` | Hover background fill for `quiet` variant |
| `--ui-button-quiet-hover-text` | `--ui-text` | Hover text color for `quiet` variant |
| `--ui-button-danger-solid-border-width` | `1px` | Transparent border width on `danger-solid` to match outlined sibling height |
| `--ui-button-danger-solid-hover-bg` | `color-mix(in oklab, var(--ui-danger) 86%, white)` | Hover background fill for `danger-solid` variant |
| `--ui-button-danger-solid-focus-shadow` | `--ui-focus-shadow` | Focus ring shadow for `danger-solid` variant |

`Segmented`:

| Token | Fallback | Used for |
| --- | --- | --- |
| `--ui-segmented-columns` | `1fr 1fr` | Grid column template for `cards` variant |
| `--ui-segmented-gap` | `10` units | Grid gap between cards (`0` in `pill` variant) |
| `--ui-segmented-item-gap` | `4` units | Gap between card label, leading icon, and detail line |
| `--ui-segmented-item-padding` | `13` and `15` units | Vertical and horizontal card padding |
| `--ui-segmented-pill-item-gap` | none | Gap between leading mark and label in `pill` variant |
| `--ui-segmented-pill-item-height` | `26` units | Segment height in `pill` variant |
| `--ui-segmented-pill-item-padding` | `0` and `4` units | Segment padding in `pill` variant |
| `--ui-segmented-item-font-size` | `14` units | Card label font size |
| `--ui-segmented-item-text` | `--ui-text-muted` | Idle card label color |
| `--ui-segmented-item-on-bg` | `12%` `--ui-accent` (`cards`) or `--ui-popover` (`pill`) | Checked segment background fill |
| `--ui-segmented-item-on-text` | `--ui-text` | Checked segment label color |
| `--ui-segmented-detail-text` | `--ui-text-faint` | Secondary `detail` line color |

`Dropdown` (note: because the popup listbox portals into `document.body`, popup-specific tokens—`--ui-dropdown-tick-text`, `--ui-dropdown-option-*`, `--ui-dropdown-list-*`, and `--ui-dropdown-font-size` on options—must be set on `:root` or `body`):

| Token | Fallback | Used for |
| --- | --- | --- |
| `--ui-dropdown-min-height` | `42` units (`28` for `quiet`) | Minimum trigger field height |
| `--ui-dropdown-max-width` | `320` units (`200` for `quiet`) | Maximum trigger field width (`100%` fills container) |
| `--ui-dropdown-padding` | `9`, `10`, `9`, `14` units | Trigger field padding |
| `--ui-dropdown-radius` | `--ui-radius-md` | Trigger field corner radius |
| `--ui-dropdown-bg` | `--ui-surface-muted` | Trigger field background fill |
| `--ui-dropdown-border` | `--ui-line-subtle` | Idle trigger field border |
| `--ui-dropdown-hover-border` | `--ui-line` | Hovered trigger field border |
| `--ui-dropdown-font-size` | `14` units | Trigger value and option label font size |
| `--ui-dropdown-tick-text` | `--ui-accent` | Selected option checkmark color |
| `--ui-dropdown-option-min-height` | `auto` | Minimum option row height |
| `--ui-dropdown-option-padding-y` | `8` units | Vertical option row padding |
| `--ui-dropdown-list-bg` | `--ui-surface` | Popup listbox background fill |
| `--ui-dropdown-list-border` | `none` | Popup listbox border shorthand (e.g. `1px solid var(--ui-line)`) |
| `--ui-dropdown-list-z-index` | `98` | Popup listbox `z-index` |
| `--ui-menu-z-index` | `50` | `DropdownMenu` popup `z-index` |

Focus styling tokens shared across `Button`, `Switch`, `Segmented`, and `Dropdown`:

| Token | Fallback | Used for |
| --- | --- | --- |
| `--ui-focus-outline` | `2px solid var(--ui-ring)` | Focus outline (`none` when using box-shadow rings) |
| `--ui-focus-border` | Current state border color | Focused control border color |
| `--ui-focus-shadow` | `none` | Focus ring `box-shadow` |

`npm run tokens:check -w @stellarr/ui` validates that both apps define every required system token, neither app defines an unknown `--ui-*` property, and every `--ui-*` variable read in `packages/stellarr-ui/src/**/*.css` is documented in the tables above.

## Behavior primitives

Custom overlays rendered by an application (such as custom modals, popovers, or context menus) can use `@stellarr/ui/behavior` to participate in the shared layer stack so `Escape`, outside pointer presses, background `inert`/`aria-hidden` state, and scroll locking coordinate across nested layers.

| Export | Description |
| --- | --- |
| `useLayer(active, layer)` | Registers an active overlay on the global layer stack. `layer` provides `node()` and optional `owns(target)`, `onEscape()`, `onTab(event)`, and `onPointerDownOutside(target)` callbacks (`Layer` type). |
| `hideOthers(keep)` | Marks all sibling children of `document.body` outside `keep` as hidden from assistive technology (reference-counted per element) and returns a cleanup function. |
| `lockScroll(areas)` | Prevents document scrolling everywhere except inside elements returned by `areas()`, and returns a nestable cleanup function. |
| `allowScroll(areas)` | Whitelists additional scrollable elements (such as a portaled `Dropdown` listbox opened inside a `Dialog`) while a `lockScroll` is active. |
| `lockPointer()` | Disables pointer events on the underlying page while a menu is open and returns a nestable cleanup function. |
| `usePresence(open)` | Keeps an element mounted until its CSS exit animation or transition finishes. Returns `{ present, ref }`. |
| `focusFirst(container)` | Moves focus to the first non-link tabbable element inside `container` (falling back to `container` itself). |
| `trapTab(event, container, options)` | Wraps `Tab` / `Shift+Tab` at the boundaries of `container` and reclaims focus from outside unless `options.reclaim` is `false`. |
| `FOCUSABLE` | CSS selector matching keyboard-focusable elements. |
| `place(anchor, size, view, options)` | Computes viewport-clamped floating coordinates with automatic axis flipping (`PlaceOptions`, `Side`, `Align`, `Placement`). |
| `measure(element)` | Returns the layout dimensions of `element` independent of CSS transforms. |
| `snap(value)` | Rounds a CSS pixel length to the nearest physical device pixel. |
| `useControllableState(options)` | Manages state that can be either controlled by props or uncontrolled internally. |
| `Slot` | Merges props onto a single child element for `asChild` composition. |
| `composeRefs(...refs)`, `useComposedRefs(...refs)` | Combines multiple React refs into a single ref callback. |

Example custom popover participating in the layer stack:

```tsx
import { useRef, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { useLayer } from "@stellarr/ui/behavior";

export function Popover({ open, onClose, children }: { open: boolean; onClose: () => void; children: ReactNode }) {
  const ref = useRef<HTMLDivElement>(null);

  useLayer(open, {
    node: () => ref.current,
    onEscape: onClose,
    onPointerDownOutside: onClose,
  });

  return open ? createPortal(<div ref={ref}>{children}</div>, document.body) : null;
}
```

## Verification

From the repository root:

```bash
npm run typecheck:ui
npm run test:ui
npm run tokens:check -w @stellarr/ui
```
