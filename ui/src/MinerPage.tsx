import { useEffect, useRef, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  minerBackend,
  type MinerInstallProgress,
  type MinerPool,
  type MinerSettings,
  type MinerStatus,
} from "./backend";
import { Tile } from "./Dashboard";
import { formatInt, formatUptime } from "./format";

const POLL_MS = 2000;
const MAX_LINES = 2000;
const RELEASE_URL = "https://github.com/karlsencoin/karlsen-miner/releases";
const DEVICE_PRESETS = [
  { value: "all", label: "All GPUs" },
  { value: "amd", label: "AMD only" },
  { value: "nvidia", label: "NVIDIA only" },
  { value: "intel", label: "Intel Arc only" },
];
const CUSTOM = "custom";

function formatMiB(bytes: number): string {
  return `${(bytes / (1024 * 1024)).toFixed(1)} MiB`;
}

function installLabel(p: MinerInstallProgress): string {
  if (p.stage !== "downloading") return "Verifying SHA-256 and installing…";
  return p.totalBytes
    ? `Downloading… ${formatMiB(p.receivedBytes)} of ${formatMiB(p.totalBytes)}`
    : `Downloading… ${formatMiB(p.receivedBytes)}`;
}

function stateLabel(status: MinerStatus): string {
  if (status.install) return "INSTALLING";
  if (!status.installed) return "NOT INSTALLED";
  // An exit Desktop will not retry (e.g. no GPU) is a failed start, not a crash loop.
  if (status.state === "crashed" && !status.restartPending) return "FAILED";
  return status.state.toUpperCase();
}

export function MinerPage() {
  const [status, setStatus] = useState<MinerStatus | null>(null);
  const [settings, setSettings] = useState<MinerSettings | null>(null);
  const [pools, setPools] = useState<MinerPool[]>([]);
  const [lines, setLines] = useState<string[]>([]);
  const [dirty, setDirty] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [customPool, setCustomPool] = useState(false);
  const [customDevices, setCustomDevices] = useState(false);
  const logBox = useRef<HTMLPreElement>(null);

  useEffect(() => {
    minerBackend.status().then(setStatus);
    minerBackend.logs().then(setLines);
    Promise.all([minerBackend.pools(), minerBackend.getSettings()]).then(([p, s]) => {
      setPools(p);
      setSettings(s);
      setCustomPool(!p.some((pool) => pool.url === s.poolUrl));
      setCustomDevices(!DEVICE_PRESETS.some((d) => d.value === s.devices));
    });
    const unStatus = minerBackend.onStatus(setStatus);
    const unInstall = minerBackend.onInstallProgress((install) =>
      setStatus((prev) => (prev ? { ...prev, install } : prev)),
    );
    const unLog = minerBackend.onLog((line) =>
      setLines((prev) => {
        const next = prev.length >= MAX_LINES ? prev.slice(prev.length - MAX_LINES + 1) : prev.slice();
        next.push(line);
        return next;
      }),
    );
    // Hashrate, shares and uptime change without a state event.
    const id = setInterval(() => minerBackend.status().then(setStatus), POLL_MS);
    return () => {
      unStatus.then((f) => f());
      unInstall.then((f) => f());
      unLog.then((f) => f());
      clearInterval(id);
    };
  }, []);

  useEffect(() => {
    if (logBox.current) logBox.current.scrollTop = logBox.current.scrollHeight;
  }, [lines]);

  if (!status || !settings) return <div className="page">Loading…</div>;

  const set = <K extends keyof MinerSettings>(k: K, v: MinerSettings[K]) => {
    setSettings({ ...settings, [k]: v });
    setDirty(true);
  };

  /** Run a backend action, showing its error text (the backend validates everything). */
  const run = async (action: () => Promise<void>) => {
    setBusy(true);
    setError(null);
    try {
      await action();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
      minerBackend.status().then(setStatus);
    }
  };

  const save = () =>
    run(async () => {
      await minerBackend.saveSettings(settings);
      setDirty(false);
    });

  const start = () =>
    run(async () => {
      await minerBackend.saveSettings(settings);
      setDirty(false);
      await minerBackend.start();
    });

  const uninstall = () => {
    if (window.confirm(`Remove KarlsenMiner v${status.version} from this computer? Your miner settings are kept.`)) {
      void run(minerBackend.uninstall);
    }
  };

  const active = status.state === "running" || status.state === "starting" || status.state === "stopping";
  // A scheduled automatic restart is also something the user can stop.
  const stoppable = active || status.restartPending;
  const installing = status.install != null;
  const percent =
    status.install?.totalBytes && status.install.stage === "downloading"
      ? Math.min(100, (status.install.receivedBytes / status.install.totalBytes) * 100)
      : installing
        ? 100
        : 0;

  return (
    <div className="page settings miner-page">
      <div className="node-header">
        <div>
          <div className="node-title">
            KarlsenMiner v{status.version} · <span className={`state-${status.state}`}>{stateLabel(status)}</span>
          </div>
          <div className="node-sub">
            GPU miner for Karlsen (KarlsenHashV2) · AMD, NVIDIA and Intel Arc · 6 GB+ VRAM
            {status.pid != null ? ` · pid ${status.pid}` : ""}
          </div>
        </div>
        <div className="actions">
          {status.installed && !stoppable && (
            <button className="primary" disabled={busy || installing} onClick={start}>Start mining</button>
          )}
          {stoppable && (
            <button disabled={busy || status.state === "stopping"} onClick={() => run(minerBackend.stop)}>Stop mining</button>
          )}
        </div>
      </div>

      {(error || status.lastError) && <div className="node-error">{error ?? status.lastError}</div>}

      {!status.supported && (
        <div className="hint">KarlsenMiner is available for Windows x64 and Linux x64 only.</div>
      )}

      {status.supported && !status.installed && (
        <section>
          <h2>Install</h2>
          <p className="help">
            KarlsenMiner is not part of the Karlsen Desktop installer. Karlsen Desktop downloads the official
            v{status.version} release from{" "}
            <a href={RELEASE_URL} onClick={(e) => { e.preventDefault(); void openUrl(RELEASE_URL); }}>
              github.com/karlsencoin/karlsen-miner
            </a>{" "}
            (about 2 MiB) and installs it only if its SHA-256 matches the value built into this version of Karlsen Desktop.
            Antivirus software often reports GPU miners as "potentially unwanted"; if yours removes the file, allow it and
            install again.
          </p>
          <div className="actions" style={{ marginTop: 10 }}>
            <button className="primary" disabled={busy || installing} onClick={() => run(minerBackend.install)}>
              {installing ? "Installing…" : `Download and install KarlsenMiner v${status.version}`}
            </button>
            {status.install && <span className="hint" style={{ marginTop: 0 }}>{installLabel(status.install)}</span>}
          </div>
          {installing && <div className="progress"><div style={{ width: `${percent}%` }} /></div>}
        </section>
      )}

      {status.installed && (
        <section>
          <div className="tiles">
            <Tile label="Hashrate" value={status.hashrateMhs != null ? `${status.hashrateMhs.toFixed(2)} MH/s` : "—"} />
            <Tile label="Accepted Shares" value={status.accepted != null ? formatInt(status.accepted) : "—"} />
            <Tile label="Rejected Shares" value={status.rejected != null ? formatInt(status.rejected) : "—"} />
            <Tile label="Uptime" value={status.uptimeSecs != null ? formatUptime(status.uptimeSecs) : "—"} />
          </div>
        </section>
      )}

      <section>
        <h2>Mining Settings</h2>
        <div className="grid2">
          <span>Mining address</span>
          <input
            value={settings.miningAddress}
            placeholder="karlsen:…"
            spellCheck={false}
            onChange={(e) => set("miningAddress", e.target.value.trim())}
          />
          <span>Rig name</span>
          <input value={settings.workerName} spellCheck={false} onChange={(e) => set("workerName", e.target.value.trim())} />
          <span>Pool</span>
          <div className="miner-row">
            <select
              value={customPool ? CUSTOM : settings.poolUrl}
              onChange={(e) => {
                const custom = e.target.value === CUSTOM;
                setCustomPool(custom);
                if (!custom) set("poolUrl", e.target.value);
              }}
            >
              {pools.map((p) => (
                <option key={p.url} value={p.url}>{p.name} ({p.payout})</option>
              ))}
              <option value={CUSTOM}>Custom pool…</option>
            </select>
            <input
              value={settings.poolUrl}
              readOnly={!customPool}
              spellCheck={false}
              placeholder="stratum+ssl://host:port"
              onChange={(e) => set("poolUrl", e.target.value.trim())}
            />
          </div>
          <span>GPUs</span>
          <div className="miner-row">
            <select
              value={customDevices ? CUSTOM : settings.devices}
              onChange={(e) => {
                const custom = e.target.value === CUSTOM;
                setCustomDevices(custom);
                if (!custom) set("devices", e.target.value);
              }}
            >
              {DEVICE_PRESETS.map((d) => (
                <option key={d.value} value={d.value}>{d.label}</option>
              ))}
              <option value={CUSTOM}>Specific cards…</option>
            </select>
            {customDevices && (
              <input
                value={settings.devices}
                spellCheck={false}
                placeholder="amd:0,1 · nvidia:0 · 0,1,3"
                onChange={(e) => set("devices", e.target.value.trim())}
              />
            )}
          </div>
          <span>Pause GPU at (°C)</span>
          <input
            type="number"
            min={50}
            max={100}
            value={settings.gpuOffTemp ?? ""}
            placeholder="off"
            onChange={(e) => set("gpuOffTemp", e.target.value === "" ? null : Number(e.target.value))}
          />
        </div>
        <p className="help">
          Rewards are paid by the pool to the mining address. Use a receive address from the WALLET tab. The address is
          checked (including its checksum) before mining starts. Pool fees and payout rules are set by each pool.
        </p>
        <label>
          <input type="checkbox" checked={settings.autoStart} onChange={(e) => set("autoStart", e.target.checked)} />
          {" "}Start mining automatically when Karlsen Desktop opens
        </label>
        <div className="actions" style={{ marginTop: 10 }}>
          <button disabled={!dirty || busy} onClick={save}>Save</button>
          {dirty && active && <span className="hint" style={{ marginTop: 0 }}>Changes apply the next time mining starts.</span>}
        </div>
      </section>

      <section>
        <h2>Miner Output</h2>
        <pre ref={logBox} className="log miner-log">
          {lines.length === 0 && <div className="hint">No output yet.</div>}
          {lines.map((l, i) => (
            <div key={i} className={l.includes("ERROR") ? "log-err" : l.startsWith("DESKTOP:") ? "log-desktop" : ""}>{l}</div>
          ))}
        </pre>
      </section>

      {status.installed && !stoppable && (
        <section>
          <h2>Maintenance</h2>
          <button className="danger" disabled={busy} onClick={uninstall}>Remove KarlsenMiner</button>
        </section>
      )}
    </div>
  );
}
