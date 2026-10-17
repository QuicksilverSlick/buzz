/**
 * WCAG contrast of Tailwind palette text on the app's default themes, for
 * tests that pin colored text at 4.5:1 (project colors, Inbox types), and
 * the OKLab distance between stripe colors on those themes.
 */
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";

import { createThemeVars, luminance } from "./adaptive-theme.ts";
import { extractThemeInfo, loadThemeData } from "./theme-loader.ts";

const tailwindTheme = readFileSync(
  createRequire(import.meta.url).resolve("tailwindcss/theme.css"),
  "utf8",
);

/** Tailwind's oklch shade ("amber-800") to linear sRGB, clipped. */
function shadeLinear(name) {
  const match = new RegExp(
    `--color-${name}:\\s*oklch\\(([\\d.]+)% ([\\d.]+) ([\\d.]+)\\)`,
  ).exec(tailwindTheme);
  assert.ok(match, `Tailwind has no ${name}`);
  const L = Number(match[1]) / 100;
  const h = (Number(match[3]) * Math.PI) / 180;
  const a = Number(match[2]) * Math.cos(h);
  const b = Number(match[2]) * Math.sin(h);
  const l = (L + 0.3963377774 * a + 0.2158037573 * b) ** 3;
  const m = (L - 0.1055613458 * a - 0.0638541728 * b) ** 3;
  const s = (L - 0.0894841775 * a - 1.291485548 * b) ** 3;
  const clip = (v) => Math.min(1, Math.max(0, v));
  const r = clip(4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s);
  const g = clip(-1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s);
  const bl = clip(-0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s);
  return [r, g, bl];
}

/** Tailwind's oklch shade ("amber-800") to relative luminance, clipped to sRGB. */
export function shadeLuminance(name) {
  const [r, g, b] = shadeLinear(name);
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

const encode = (c) =>
  c <= 0.0031308 ? 12.92 * c : 1.055 * c ** (1 / 2.4) - 0.055;
const decode = (c) => (c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4);

/**
 * A shade at `alpha` over a 0-255 sRGB background, blended in sRGB as the
 * browser does, in OKLab.
 */
function blendOklab(shade, alpha, background) {
  const [r, g, b] = shadeLinear(shade).map((c, i) =>
    decode(encode(c) * alpha + (background[i] / 255) * (1 - alpha)),
  );
  const l = Math.cbrt(0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b);
  const m = Math.cbrt(0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b);
  const s = Math.cbrt(0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b);
  return [
    0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s,
    1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s,
    0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s,
  ];
}

/** A theme var ("H S% L%") to 0-255 sRGB channels. */
function hslRgb(value) {
  const [h, s, l] = value.split(" ").map((part) => Number.parseFloat(part));
  const k = (n) => (n + h / 30) % 12;
  const amp = (s / 100) * Math.min(l / 100, 1 - l / 100);
  return [0, 8, 4].map((n) =>
    Math.round(
      255 * (l / 100 - amp * Math.max(-1, Math.min(k(n) - 3, 9 - k(n), 1))),
    ),
  );
}

function rgbLuminance(rgb) {
  return luminance(
    `#${rgb.map((c) => Math.round(c).toString(16).padStart(2, "0")).join("")}`,
  );
}

export function contrast(a, b) {
  return (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);
}

export const DEFAULT_THEMES = [
  "buzz",
  "buzz-dark",
  "catppuccin-latte",
  "catppuccin-macchiato",
];

/**
 * Each default theme's text surfaces as luminances: background, muted,
 * popover, and the Inbox's selected row (`color-mix` in sRGB, 70%
 * background and 30% muted).
 */
export async function themeSurfaces() {
  const themes = [];
  for (const name of DEFAULT_THEMES) {
    const info = extractThemeInfo(name, await loadThemeData(name));
    const { isDark, vars } = createThemeVars(info.bg, info.fg, info.comment);
    const background = hslRgb(vars["--background"]);
    const muted = hslRgb(vars["--muted"]);
    themes.push({
      name,
      isDark,
      background,
      surfaces: {
        "--background": rgbLuminance(background),
        "--muted": rgbLuminance(muted),
        "--popover": rgbLuminance(hslRgb(vars["--popover"])),
        "selected row": rgbLuminance(
          background.map((c, i) => c * 0.7 + muted[i] * 0.3),
        ),
      },
    });
  }
  return themes;
}

/** Fails naming every shade under 4.5:1 on any default theme surface. */
export async function assertTextContrast(hues) {
  const failures = [];
  for (const { name, isDark, surfaces } of await themeSurfaces()) {
    for (const hue of hues) {
      const shade = `${hue}-${isDark ? 300 : 800}`;
      for (const [surface, back] of Object.entries(surfaces)) {
        const ratio = contrast(shadeLuminance(shade), back);
        if (ratio < 4.5) {
          failures.push(`${shade} on ${name} ${surface}: ${ratio.toFixed(2)}`);
        }
      }
    }
  }
  assert.deepEqual(failures, []);
}

/** "border-l-lime-600 dark:border-l-lime-400" or "border-l-amber-500/70" in one theme. */
function stripeShade(classes, isDark) {
  const parts = classes.split(" ");
  const own = parts.find((part) => !part.startsWith("dark:"));
  const dark = parts.find((part) => part.startsWith("dark:"));
  const match = /^(?:dark:)?border-l-([a-z]+-\d+)(?:\/(\d+))?$/.exec(
    (isDark && dark) || own,
  );
  assert.ok(match, `not a stripe class: ${classes}`);
  return { shade: match[1], alpha: match[2] ? Number(match[2]) / 100 : 1 };
}

/**
 * Fails naming every stripe in `stripes` closer than `min` (OKLab) to one
 * in `others` on any default theme's background, at their real alpha.
 */
export async function assertStripesApart(stripes, others, min) {
  const failures = [];
  for (const { name, isDark, background } of await themeSurfaces()) {
    const lab = (classes) => {
      const { shade, alpha } = stripeShade(classes, isDark);
      return blendOklab(shade, alpha, background);
    };
    for (const stripe of stripes) {
      for (const other of others) {
        const [a, b] = [lab(stripe), lab(other)];
        const distance = Math.hypot(...a.map((v, i) => v - b[i]));
        if (distance < min) {
          failures.push(
            `${stripe} ~ ${other} on ${name}: ${distance.toFixed(3)}`,
          );
        }
      }
    }
  }
  assert.deepEqual(failures, []);
}
