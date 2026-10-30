// Line symbols a space can use as its icon, stored as `sym:<name>` (see
// DEFAULT_SPACE_ICONS in src-tauri/src/model.rs) and drawn in the space's
// color. Paths are 16×16, stroked like the sidebar's other icons.
export const SYMBOLS: Record<string, string> = {
  home: "M2.5 7.5 8 3l5.5 4.5M4 6.5V13h3v-3h2v3h3V6.5",
  briefcase: "M2.5 5.5h11v7h-11zM6 5.5V4h4v1.5M2.5 9h11",
  code: "M5.5 4.5 2 8l3.5 3.5M10.5 4.5 14 8l-3.5 3.5M9 3 7 13",
  book: "M8 4.5C6.5 3.4 4.5 3 2.5 3.5v9c2-.5 4-.1 5.5 1 1.5-1.1 3.5-1.5 5.5-1v-9c-2-.5-4-.1-5.5 1zM8 4.5v9",
  star: "m8 2.5 1.7 3.5 3.8.5-2.8 2.6.7 3.8L8 11.1l-3.4 1.8.7-3.8-2.8-2.6 3.8-.5z",
  leaf: "M3 13c0-6 4-9.5 10-10 0 6-3.5 10-9.5 10M3 13l5-5",
  bolt: "M9 2 3.5 9H8l-1 5 5.5-7H8z",
  globe: "M8 2a6 6 0 1 0 0 12A6 6 0 0 0 8 2ZM2 8h12M8 2c-2 2-2 10 0 12M8 2c2 2 2 10 0 12",
  terminal: "M2.5 3h11v10h-11zM5 6.5 7 8 5 9.5M8.5 10H11",
  flask: "M6 2h4M6.5 2v4L3 13h10L9.5 6V2M4.6 10h6.8",
  chart: "M2.5 13.5h11M4.5 11V8M8 11V4.5M11.5 11V6.5",
  calendar: "M2.5 3.5h11v10h-11zM2.5 6.5h11M5.5 2v3M10.5 2v3",
  mail: "M2 4h12v8.5H2zM2 4.5l6 4.5 6-4.5",
  chat: "M2.5 3.5h11V11H7l-3 2.5V11H2.5z",
  cart: "M1.5 2.5h2L5 10h7l1.5-5.5H4.3M6 13h.01M11.5 13h.01",
  graduation: "M1.5 6 8 3l6.5 3L8 9zM4 7.2v3.3c1 1 2.5 1.5 4 1.5s3-.5 4-1.5V7.2M14.5 6v3.5",
  palette:
    "M8 2a6 6 0 0 0 0 12c1 0 1.5-.6 1.5-1.3 0-1-.8-1.2-.8-2 0-.7.6-1.2 1.3-1.2H12a2 2 0 0 0 2-2C14 4.5 11.3 2 8 2ZM5 8h.01M6 5h.01M9.5 4.5h.01",
  music: "M6 12V3.5l7-1.5v8.5M6 12a1.75 1.75 0 1 1-3.5 0A1.75 1.75 0 0 1 6 12Zm7-1.5a1.75 1.75 0 1 1-3.5 0 1.75 1.75 0 0 1 3.5 0Z",
  camera: "M2.5 5.5H5L6 4h4l1 1.5h2.5v7h-11zM8 7a2 2 0 1 0 0 4 2 2 0 0 0 0-4Z",
  gamepad:
    "M4.5 5h7a3 3 0 0 1 3 3v.5a2.5 2.5 0 0 1-4.3 1.7l-.7-.7h-3l-.7.7a2.5 2.5 0 0 1-4.3-1.7V8a3 3 0 0 1 3-3ZM4.5 7v2M3.5 8h2M11 7.5h.01M12 9h.01",
  heart: "M8 13.5S2.5 10.2 2.5 6.3A2.8 2.8 0 0 1 8 5a2.8 2.8 0 0 1 5.5 1.3c0 3.9-5.5 7.2-5.5 7.2Z",
  coffee: "M3 6h8v4a3 3 0 0 1-3 3H6a3 3 0 0 1-3-3zm8 1h1a1.5 1.5 0 0 1 0 3h-1M5.5 2.5V4M8.5 2.5V4",
  plane: "M14 2 2 7l5 2 2 5zM7 9l3-3",
  lock: "M3.5 7.5h9v6h-9zM5.5 7.5V5a2.5 2.5 0 0 1 5 0v2.5",
  moon: "M13 9.5A5.5 5.5 0 1 1 6.5 3 4.5 4.5 0 0 0 13 9.5Z",
  sun: "M8 5.5a2.5 2.5 0 1 0 0 5 2.5 2.5 0 0 0 0-5ZM8 1.5V3M8 13v1.5M1.5 8H3M13 8h1.5M3.4 3.4l1 1M11.6 11.6l1 1M3.4 12.6l1-1M11.6 4.4l1-1",
};

/** The path for a `sym:` icon, or `null` for an emoji (or no icon). */
export function symbolPath(icon: string | null): string | null {
  return icon?.startsWith("sym:") ? (SYMBOLS[icon.slice(4)] ?? null) : null;
}
