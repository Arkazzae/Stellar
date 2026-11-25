// Relative imports only: the build reads this module from vite.config.ts,
// where the app's `@/` alias does not resolve.
import {
  CONTRASTS,
  CUSTOM_PALETTE,
  DEFAULT_THEME,
  DENSITIES,
  LIGHT_TINT_MAX,
  PALETTES,
  PALETTE_ROLE_VARIABLES,
  SCHEMES,
  SIZES,
  THEME_STORAGE_KEY,
  paletteCssVariables,
} from "./themes";

const ids = (list: readonly { id: string }[]) => list.map(({ id }) => id);

/** Each preset's variables, by id; worked out here, at build time, not in the app. */
const PALETTE_CSS_VARIABLES = Object.fromEntries(PALETTES.map((palette) => [palette.id, paletteCssVariables(palette)]));

/**
 * Runs before paint, so the first frame is already in the user's theme. The
 * build writes it inline into the `<head>` of the web client's document
 * (`web/index.html`); without it a light-scheme user would see one dark
 * frame on every load.
 *
 * It repeats the narrowing in `themes.ts` instead of importing it, because an
 * import would pull the whole module into a blocking inline script. Everything
 * it needs to decide is injected from that module as data when the document is
 * built, so the two cannot disagree about what is valid or what the defaults
 * are, and the script only has to be kept in step with `applyThemeAttributes`
 * in how it writes them.
 *
 * A custom palette is checked the way `normaliseCustomTheme` checks it: every
 * role a hex colour, its stored derived values in range, or none of it used.
 *
 * The face and the wallpaper are not written here: they belong to the
 * signed-in app, which writes them as it mounts (`AppAppearance`), so the
 * sign-in pages that fetch it ahead stay in Outfit and opaque.
 */
export const THEME_SCRIPT = `(function(){
var DEFAULT=${JSON.stringify(DEFAULT_THEME)},VARIABLES=${JSON.stringify(PALETTE_CSS_VARIABLES)};
var PALETTES=${JSON.stringify(ids(PALETTES))},SIZES=${JSON.stringify(ids(SIZES))},DENSITIES=${JSON.stringify(ids(DENSITIES))};
var SCHEMES=${JSON.stringify(SCHEMES)},CONTRASTS=${JSON.stringify(CONTRASTS)},ROLES=${JSON.stringify(PALETTE_ROLE_VARIABLES)};
var OPAQUE=/^#[\\da-f]{6}$/i,HEX=/^#[\\da-f]{6}([\\da-f]{2})?$/i,saved=null;
try{var raw=localStorage.getItem(${JSON.stringify(THEME_STORAGE_KEY)});if(raw)saved=JSON.parse(raw)}catch(e){}
function pick(key,allowed){
var value=saved&&saved[key];
return allowed.indexOf(value)<0?DEFAULT[key]:value;
}
function onColour(hex){
var c=[1,3,5].map(function(i){var n=parseInt(hex.slice(i,i+2),16)/255;return n<=.04045?n/12.92:Math.pow((n+.055)/1.055,2.4)});
var l=.2126*c[0]+.7152*c[1]+.0722*c[2];
return(l+.05)/.05>=1.05/(l+.05)?"#000000":"#ffffff";
}
function tint(v,max){return typeof v==="number"&&isFinite(v)&&v>=0&&v<=max}
function custom(c){
var colors=c&&c.colors,neutral=c&&c.neutral,faint=c&&c.accessibleFaint,out={},i,v;
if(!colors||typeof colors.accent!=="string"||!OPAQUE.test(colors.accent))return null;
if(typeof faint!=="string"||!OPAQUE.test(faint)||!neutral||!tint(neutral.hue,359)||!tint(neutral.chroma,${LIGHT_TINT_MAX}))return null;
for(i=0;i<ROLES.length;i++){
v=colors[ROLES[i][0]];v=v&&v[ROLES[i][1]];
if(typeof v!=="string"||!HEX.test(v))return null;
out[ROLES[i][2]]=v.toLowerCase();
}
v=colors.accent.toLowerCase();
out["--palette-accent"]=v;
out["--palette-on-accent"]=onColour(v);
out["--palette-on-background"]=onColour(out["--palette-background-primary"]);
out["--palette-on-danger"]=onColour(out["--palette-danger"]);
out["--palette-text-faint-accessible"]=faint.toLowerCase();
out["--h-neutral"]=String(neutral.hue);
out["--c-neutral"]=String(neutral.chroma);
return out;
}
var own=saved&&saved.palette===${JSON.stringify(CUSTOM_PALETTE)}?custom(saved.custom):null;
var palette=own?${JSON.stringify(CUSTOM_PALETTE)}:pick("palette",PALETTES),variables=own||Object.assign({},VARIABLES[palette]),accent=saved&&saved.accent;
if(typeof accent==="string"&&OPAQUE.test(accent)){
accent=accent.toLowerCase();
variables["--palette-accent"]=accent;
variables["--palette-on-accent"]=onColour(accent);
}
var root=document.documentElement,scheme=pick("scheme",SCHEMES);
Object.keys(variables).forEach(function(name){root.style.setProperty(name,variables[name])});
if(scheme==="system")scheme=matchMedia("(prefers-color-scheme: dark)").matches?"dark":"light";
root.dataset.theme=palette;
root.dataset.scheme=scheme;
root.dataset.contrast=pick("contrast",CONTRASTS);
root.dataset.size=pick("size",SIZES);
root.dataset.density=pick("density",DENSITIES);
})()`;
