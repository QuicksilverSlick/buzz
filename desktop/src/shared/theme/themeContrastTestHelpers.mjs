/**
 * WCAG contrast of Tailwind palette text on the app's default themes, for
 * tests that pin colored text at 4.5:1 (project colors, Inbox types).
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

/** Tailwind's oklch shade ("amber-800") to relative luminance, clipped to sRGB. */
export function shadeLuminance(name) {
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
  return 0.2126 * r + 0.7152 * g + 0.0722 * bl;
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
