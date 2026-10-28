// Space colors (ids match SPACE_COLORS in src-tauri/src/model.rs). Each
// becomes a hue/saturation pair that theme.css turns into the sidebar tint.
const HUES: Record<string, [hue: number, saturation: number]> = {
  blue: [220, 80],
  purple: [268, 70],
  pink: [330, 75],
  red: [2, 75],
  orange: [26, 85],
  yellow: [46, 85],
  green: [140, 55],
  teal: [180, 60],
  gray: [220, 0],
};

/** CSS variables for a space's tint: `--hue` and `--sat`. */
export function spaceStyle(color: string): string {
  const [hue, sat] = HUES[color] ?? HUES.blue;
  return `--hue: ${hue}; --sat: ${sat}%`;
}

/** A solid swatch for the space switcher. */
export function spaceSwatch(color: string): string {
  const [hue, sat] = HUES[color] ?? HUES.blue;
  return `hsl(${hue} ${sat}% 58%)`;
}
