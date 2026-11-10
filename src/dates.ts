// Utilidades de fechas en hora local (los inputs date/time trabajan en local).
const pad = (n: number) => String(n).padStart(2, "0");

export const dateStr = (d: Date) => `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;

export const timeStr = (d: Date) => `${pad(d.getHours())}:${pad(d.getMinutes())}`;

export function addDays(d: Date, n: number): Date {
  const r = new Date(d);
  r.setDate(r.getDate() + n);
  return r;
}

/** Suma días a una fecha `YYYY-MM-DD` sin pasar por UTC. */
export function addDaysStr(s: string, n: number): string {
  const [y, m, d] = s.slice(0, 10).split("-").map(Number);
  return dateStr(new Date(y, m - 1, d + n));
}

/** Próxima hora en punto (para "Nuevo evento"). */
export function nextHour(): Date {
  const d = new Date();
  d.setMinutes(0, 0, 0);
  d.setHours(d.getHours() + 1);
  return d;
}

/** Color de texto legible sobre un fondo hexadecimal. */
export function contrastText(hex: string): string {
  const m = /^#?([0-9a-f]{6})$/i.exec(hex.trim());
  if (!m) return "#fff";
  const n = parseInt(m[1], 16);
  const [r, g, b] = [(n >> 16) & 255, (n >> 8) & 255, n & 255];
  const lum = (0.299 * r + 0.587 * g + 0.114 * b) / 255;
  return lum > 0.65 ? "#1d2130" : "#ffffff";
}
