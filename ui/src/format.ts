export function formatUptime(secs: number | null): string {
  if (secs == null) return "—";
  const d = Math.floor(secs / 86400);
  const h = Math.floor((secs % 86400) / 3600);
  const m = Math.floor((secs % 3600) / 60);
  const s = secs % 60;
  const hms = [h, m, s].map((n) => String(n).padStart(2, "0")).join(":");
  return d > 0 ? `${d}d ${hms}` : hms;
}

export function formatHashrate(hps: bigint | null): string {
  if (hps == null) return "N/A";
  const units = ["H/s", "KH/s", "MH/s", "GH/s", "TH/s", "PH/s"];
  let v = Number(hps);
  let i = 0;
  while (v >= 1000 && i < units.length - 1) {
    v /= 1000;
    i++;
  }
  return `${v.toFixed(2)} ${units[i]}`;
}

export function formatInt(n: bigint | number): string {
  return n.toLocaleString("en-US");
}

export function formatMedianTime(ms: bigint): string {
  return new Date(Number(ms)).toISOString().replace("T", " ").replace("Z", " UTC");
}
