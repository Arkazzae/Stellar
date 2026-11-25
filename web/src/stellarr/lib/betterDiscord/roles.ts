import { deriveFaint, flatten, parseHex, relativeLuminance, toHex, type Rgba } from "@/design/colour";
import { getPalette, readableTextOn, type ColourGroup, type PaletteColors } from "@/design/themes";

/**
 * Which of Discord's variables paints what our palette's roles paint, in
 * the order they are tried: the 2025 visual-refresh names first, then the
 * names before it, then the ones some themes set instead. Checked against
 * midnight's remap block, which says what each variable colours.
 */
export const DISCORD_ROLES = {
  accent: [
    "--brand-500", "--brand-experiment", "--control-brand-foreground", "--text-brand", "--brand-560",
    "--dplus-accent-ui", "--main-theme-color", "--darkplus-sec", "--themeColor1", "--mainColor",
    "--primary1-colour", "--ocean-color1", "--accent", "--primary-color", "--text-link",
  ],
  backgrounds: {
    // The frame around everything: the server list, the title bar.
    darkest: ["--background-base-lowest", "--background-tertiary", "--app-frame-background", "--app-background", "--app-bg", "--darkplus-bg", "--backgroundColor02", "--stellarr-frame"],
    // The channel list and the member list.
    dark: ["--background-base-lower", "--background-secondary", "--sidebar-panel-color", "--bg-secondary", "--darkplus-bg2", "--stellarr-sidebar"],
    // The chat itself.
    primary: ["--background-base-low", "--chat-background-default", "--background-primary", "--chat-color", "--app-foreground", "--dplus-bgc-ui-base", "--bg-primary", "--darkplus-bg", "--backgroundColor01", "--ocean-bg", "--stellarr-chat"],
    secondary: ["--background-base-lower", "--background-secondary", "--sidebar-panel-color", "--bg-secondary", "--darkplus-bg2", "--stellarr-sidebar"],
    // Fields: the message box and inputs.
    tertiary: ["--input-background-default", "--channeltextarea-background", "--background-surface-high", "--background-secondary-alt", "--main-textarea-color", "--textbox-background", "--textarea-background", "--message-box-transparency", "--stellarr-input"],
    hover: ["--background-mod-subtle", "--interactive-background-hover", "--background-modifier-hover", "--message-color-hover", "--hover-color"],
    active: ["--background-mod-normal", "--interactive-background-selected", "--background-modifier-selected", "--background-modifier-active"],
    // Popouts, menus and cards.
    floating: ["--background-surface-higher", "--background-surface-high", "--background-floating", "--card-background-default", "--popout-color", "--dplus-bgc-popout", "--tooltip-color"],
  },
  text: {
    normal: ["--text-default", "--text-normal", "--chat-font-color", "--text-color", "--xp-txt-color", "--header-primary", "--stellarr-message"],
    // Channel names, icons, secondary lines.
    muted: ["--text-subtle", "--channels-default", "--interactive-normal", "--interactive-text-default", "--header-secondary", "--chat-subtext-color", "--inactivetext-colour"],
    // Timestamps and muted channels: Discord's dimmest tier.
    faint: ["--text-muted", "--channel-text-area-placeholder", "--interactive-muted", "--input-placeholder-color"],
  },
  borders: {
    subtle: ["--border-subtle", "--border-muted", "--background-modifier-accent", "--separator-color", "--border-color"],
    default: ["--border-normal", "--border-subtle", "--background-modifier-accent", "--separator-color", "--border-color"],
    strong: ["--border-strong", "--border-normal"],
  },
  semantic: {
    success: ["--status-positive", "--text-feedback-positive", "--status-online", "--green-360", "--rs-online-color", "--green-color", "--online-color"],
    warning: ["--status-warning", "--text-feedback-warning", "--status-idle", "--yellow-300", "--rs-idle-color", "--yellow-color", "--idle-color"],
    danger: ["--status-danger", "--text-feedback-critical", "--status-dnd", "--red-400", "--rs-dnd-color", "--red-color", "--dnd-color"],
    info: ["--text-feedback-info", "--text-link", "--info-help-foreground"],
  },
} as const;

/** Shading over the wallpaper in themes that set a transparent chat surface. */
export const SHADING_VARIABLES = ["--background-shading", "--middle-shading", "--theme-transparency", "--backgroundColor"] as const;

/** Every variable the table may read, for the evaluator to resolve. */
export const DISCORD_VARIABLES: readonly string[] = [
  ...new Set([
    ...DISCORD_ROLES.accent,
    ...Object.values(DISCORD_ROLES.backgrounds).flat(),
    ...Object.values(DISCORD_ROLES.text).flat(),
    ...Object.values(DISCORD_ROLES.borders).flat(),
    ...Object.values(DISCORD_ROLES.semantic).flat(),
    ...SHADING_VARIABLES,
  ]),
];

/** A role and where its colour came from: the theme's variable, or our own derivation. */
export interface RoleSource {
  group: ColourGroup | "accent";
  role: string;
  /** The Discord variable it was read from, or undefined when it was filled in. */
  from?: string;
}

export interface MappedTheme {
  /** A full palette, or undefined when the core of one did not come across. */
  colors?: PaletteColors;
  accent?: string;
  roles: RoleSource[];
  /** The core roles that did not come across, when there is no palette. */
  missingCore: string[];
  /** The backgrounds are the theme's shading over its wallpaper, not colours it set. */
  backgroundsFromShading?: boolean;
}

/** Resolved colours of Discord's variables, by name; a missing one was not set. */
export type ResolvedVariables = ReadonlyMap<string, Rgba>;

/** A colour for a role and the variable it was read from; no name when it is ours. */
interface Found {
  name?: string;
  colour: Rgba;
}

const BLACK: Rgba = [0, 0, 0, 1];
const opaque = (colour: Rgba, under: Rgba) => (colour[3] >= 1 ? colour : flatten(colour, under));
const mix = (a: Rgba, b: Rgba, amount: number): Rgba => [
  a[0] + (b[0] - a[0]) * amount,
  a[1] + (b[1] - a[1]) * amount,
  a[2] + (b[2] - a[2]) * amount,
  1,
];
const withAlpha = (colour: Rgba, alpha: number): Rgba => [colour[0], colour[1], colour[2], alpha];

/**
 * Keep the paint the theme defines, then fill in neighboring roles from a
 * real surface or wallpaper. A theme that only changes accent or type
 * stays a partial import.
 */
export function mapDiscordTheme(resolved: ResolvedVariables, withWallpaper = false): MappedTheme {
  const roles: RoleSource[] = [];
  const pick = (names: readonly string[]): Found | undefined => {
    for (const name of names) {
      const colour = resolved.get(name);
      if (colour !== undefined && colour[3] > 0) return { name, colour };
    }
    return undefined;
  };

  const accent = pick(DISCORD_ROLES.accent);
  const accentHex = accent === undefined ? undefined : toHex(opaque(accent.colour, BLACK).slice(0, 3));

  // A picture theme leaves the chat's background unset or transparent: what
  // is there is the picture under its shading, or under black, which is then
  // ours and said to be filled in.
  const chat = pick(DISCORD_ROLES.backgrounds.primary);
  const fromShading = withWallpaper && chat === undefined;
  const shading: Found | undefined = fromShading ? (pick(SHADING_VARIABLES) ?? { colour: BLACK }) : undefined;
  const darkest = pick(DISCORD_ROLES.backgrounds.darkest) ?? shading;
  const sidebar = pick(DISCORD_ROLES.backgrounds.dark);
  const primary = chat ?? shading;
  const normal = pick(DISCORD_ROLES.text.normal);
  const missingCore = [
    ...(primary === undefined ? ["backgrounds.primary"] : []),
    ...(darkest === undefined && sidebar === undefined ? ["backgrounds.darkest"] : []),
    ...(normal === undefined ? ["text.normal"] : []),
  ];
  const surface = primary ?? sidebar ?? darkest;
  if (surface === undefined) {
    if (accentHex !== undefined) roles.push({ group: "accent", role: "accent", from: accent?.name });
    return { ...(accentHex === undefined ? {} : { accent: accentHex }), roles, missingCore };
  }

  // The frame first, onto black; everything else onto the frame.
  const frame = opaque((darkest ?? sidebar ?? surface).colour, BLACK);
  const room = opaque(surface.colour, frame);
  const fallback = getPalette("midnight").colors;
  const colours: Record<string, Record<string, string>> = { backgrounds: {}, text: {}, borders: {}, semantic: {} };
  const take = (group: ColourGroup, role: string, found: Found | undefined, filled: () => Rgba, under: Rgba) => {
    const colour = found === undefined ? filled() : group === "borders" ? found.colour : opaque(found.colour, under);
    colours[group]![role] = toHex(group === "borders" ? [...colour] : [...colour.slice(0, 3)]);
    roles.push({ group, role, ...(found?.name === undefined ? {} : { from: found.name }) });
  };

  const roomHex = toHex(room);
  const safeText = readableTextOn(roomHex);
  const textColour = normal === undefined
    ? parseHex(safeText)
    : parseHex(deriveFaint(toHex(normal.colour), safeText, [roomHex]));
  const lighter = relativeLuminance(textColour) > relativeLuminance(room);
  const step = (amount: number): Rgba => mix(room, textColour, amount);

  take("backgrounds", "darkest", darkest, () => frame, BLACK);
  take("backgrounds", "dark", sidebar, () => mix(frame, room, 0.5), frame);
  take("backgrounds", "primary", primary, () => room, frame);
  take("backgrounds", "secondary", pick(DISCORD_ROLES.backgrounds.secondary), () => mix(frame, room, 0.5), frame);
  take("backgrounds", "tertiary", pick(DISCORD_ROLES.backgrounds.tertiary), () => step(0.04), room);
  take("backgrounds", "hover", pick(DISCORD_ROLES.backgrounds.hover), () => step(0.06), room);
  take("backgrounds", "active", pick(DISCORD_ROLES.backgrounds.active), () => step(0.12), room);
  take("backgrounds", "floating", pick(DISCORD_ROLES.backgrounds.floating), () => frame, frame);

  take("text", "normal", normal === undefined ? undefined : { ...normal, colour: textColour }, () => textColour, room);
  take("text", "muted", pick(DISCORD_ROLES.text.muted), () => step(0.75), room);
  take("text", "faint", pick(DISCORD_ROLES.text.faint), () => step(0.5), room);

  const line = lighter ? ([1, 1, 1, 1] as Rgba) : BLACK;
  take("borders", "subtle", pick(DISCORD_ROLES.borders.subtle), () => withAlpha(line, 0.06), room);
  take("borders", "default", pick(DISCORD_ROLES.borders.default), () => withAlpha(line, 0.12), room);
  take("borders", "strong", pick(DISCORD_ROLES.borders.strong), () => withAlpha(line, 0.2), room);

  for (const role of ["success", "warning", "danger", "info"] as const) {
    take("semantic", role, pick(DISCORD_ROLES.semantic[role]), () => {
      const [r = 0, g = 0, b = 0] = [1, 3, 5].map((at) => Number.parseInt(fallback.semantic[role].slice(at, at + 2), 16) / 255);
      return [r, g, b, 1];
    }, room);
  }

  const themeAccent = accentHex ?? fallback.accent;
  roles.push({ group: "accent", role: "accent", ...(accent === undefined ? {} : { from: accent.name }) });
  return {
    colors: { accent: themeAccent, ...colours } as unknown as PaletteColors,
    // Only an accent the theme set is offered as its accent.
    ...(accentHex === undefined ? {} : { accent: accentHex }),
    roles,
    missingCore: [],
    ...(fromShading ? { backgroundsFromShading: true } : {}),
  };
}
