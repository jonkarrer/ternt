// Sizes and times for the Downloads box.

/** "4.2 MB", as Finder counts (1000s). */
export function formatBytes(bytes: number): string {
  if (bytes < 1000) return `${bytes} bytes`;
  const units = ["KB", "MB", "GB", "TB"];
  let value = bytes / 1000;
  let unit = 0;
  while (value >= 1000 && unit < units.length - 1) {
    value /= 1000;
    unit++;
  }
  return `${value < 10 ? value.toFixed(1) : Math.round(value)} ${units[unit]}`;
}

/** "4.2 of 12 MB", or "4.2 MB" when the size isn't known. */
export function formatProgress(received: number, total: number | null): string {
  if (total == null) return formatBytes(received);
  const whole = formatBytes(total);
  const unit = whole.split(" ")[1];
  const part = formatBytes(received);
  return part.endsWith(` ${unit}`) ? `${part.split(" ")[0]} of ${whole}` : `${part} of ${whole}`;
}

/** "12 s left", "3 min left". */
export function formatLeft(seconds: number): string {
  if (!isFinite(seconds) || seconds <= 0) return "";
  if (seconds < 60) return `${Math.max(1, Math.round(seconds))} s left`;
  if (seconds < 3600) return `${Math.round(seconds / 60)} min left`;
  return `${Math.round(seconds / 3600)} h left`;
}
