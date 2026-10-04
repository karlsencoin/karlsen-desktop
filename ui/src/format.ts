export const SOMPI_PER_KLS = 100_000_000n;

export function formatUptime(secs: number | null): string {
  if (secs == null) return "—";
  const d = Math.floor(secs / 86400);
  const h = Math.floor((secs % 86400) / 3600);
  const m = Math.floor((secs % 3600) / 60);
  const s = secs % 60;
  const hms = [h, m, s].map((n) => String(n).padStart(2, "0")).join(":");
  return d > 0 ? `${d}d ${hms}` : hms;
}

/** Scales a value into SI units: 7.47e9 -> "7.47 G". */
function siScale(v: number, units: string[]): string {
  let i = 0;
  while (Math.abs(v) >= 1000 && i < units.length - 1) {
    v /= 1000;
    i++;
  }
  return `${v.toFixed(2)} ${units[i]}`.trimEnd();
}

export function formatHashrate(hps: bigint | null): string {
  if (hps == null) return "N/A";
  return siScale(Number(hps), ["H/s", "KH/s", "MH/s", "GH/s", "TH/s", "PH/s", "EH/s"]);
}

export function formatDifficulty(d: number): string {
  return siScale(d, ["", "K", "M", "G", "T", "P", "E"]);
}

export function formatInt(n: bigint | number): string {
  return n.toLocaleString("en-US");
}

/** Local date and time without milliseconds, e.g. "2026-10-03 23:11:47". */
export function formatLocalDateTime(ms: number): string {
  const d = new Date(ms);
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`;
}

export function formatMedianTime(ms: bigint): string {
  return formatLocalDateTime(Number(ms));
}

/** Full UTC timestamp, used as a tooltip next to local times. */
export function formatUtc(ms: number): string {
  return new Date(ms).toISOString().replace("T", " ").replace("Z", " UTC");
}

/** Sompi amount as KLS with thousands separators, rounded: 195123655500n -> "1,951.24 KLS". */
export function formatKls(sompi: bigint, decimals = 2): string {
  const scale = 10n ** BigInt(decimals);
  const rounded = (sompi * scale + SOMPI_PER_KLS / 2n) / SOMPI_PER_KLS;
  const whole = formatInt(rounded / scale);
  if (decimals === 0) return `${whole} KLS`;
  return `${whole}.${(rounded % scale).toString().padStart(decimals, "0")} KLS`;
}

/** Share of `part` in `total` as a percentage with two decimals. */
export function formatPercent(part: bigint, total: bigint): string {
  if (total === 0n) return "—";
  return `${(Number((part * 1_000_000n) / total) / 10_000).toFixed(2)} %`;
}

/** Human duration: 2_511_508 -> "29 days 1 hour"; short spans in minutes/seconds. */
export function formatDuration(secs: number): string {
  const plural = (n: number, w: string) => `${n} ${w}${n === 1 ? "" : "s"}`;
  const d = Math.floor(secs / 86400);
  const h = Math.floor((secs % 86400) / 3600);
  const m = Math.floor((secs % 3600) / 60);
  if (d > 0) return h > 0 ? `${plural(d, "day")} ${plural(h, "hour")}` : plural(d, "day");
  if (h > 0) return m > 0 ? `${plural(h, "hour")} ${plural(m, "minute")}` : plural(h, "hour");
  if (m > 0) return plural(m, "minute");
  return plural(Math.max(0, Math.round(secs)), "second");
}

/** Fee rate of a bucket, e.g. "1.00 sompi/gram". */
export function formatFeeRate(b: { feerate: number } | null): string {
  return b ? `${b.feerate.toFixed(2)} sompi/gram` : "N/A";
}

/** Expected inclusion time of a bucket, e.g. "Inclusion in under 1 second". */
export function formatFeeTime(b: { estimatedSeconds: number } | null): string | undefined {
  if (!b) return undefined;
  return b.estimatedSeconds < 1 ? "Inclusion in under 1 second" : `Inclusion in ~${formatDuration(b.estimatedSeconds)}`;
}
