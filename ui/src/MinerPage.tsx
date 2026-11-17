import { useEffect, useRef, useState } from "react";
import { backend, minerBackend, type MinerSettings, type MinerStatus } from "./backend";
import { Tile } from "./Dashboard";
import { formatDuration, formatHashrate, formatInt, formatKls, formatUptime } from "./format";
import { NodeRpc } from "./rpc";

const POLL_MS = 2000;
const NODE_POLL_MS = 10_000;
const MAX_LINES = 3000;
const DEVICE_PRESETS = [
  { value: "all", label: "All GPUs" },
  { value: "amd", label: "AMD only" },
  { value: "nvidia", label: "NVIDIA only" },
  { value: "intel", label: "Intel Arc only" },
];
const CUSTOM = "custom";

/** What the miner needs to know about the local node. */
interface NodeInfo {
  isSynced: boolean;
  networkHashesPerSecond: bigint | null;
  /** Reward of the next block in sompi. */
  blockRewardSompi: number | null;
}

function stateLabel(status: MinerStatus): string {
  switch (status.state) {
    case "starting":
      return "STARTING GPUS";
    case "waiting":
      return "WAITING FOR NODE SYNC";
    case "running":
      return "MINING";
    case "crashed":
      // An exit Desktop will not retry (e.g. no GPU) is a failed start, not a crash loop.
      return status.restartPending ? "RESTARTING" : "FAILED";
    default:
      return status.state.toUpperCase();
  }
}

/**
 * Average time between blocks found at this hashrate. Karlsen mainnet produces one block per
 * second, so a miner with share s of the network hashrate finds one block every 1/s seconds
 * on average. Solo mining is luck: the real gaps vary widely around this average.
 */
function averageSecondsPerBlock(hashrateMhs: number | null, networkHps: bigint | null): number | null {
  if (!hashrateMhs || !networkHps || networkHps <= 0n) return null;
  return Number(networkHps) / (hashrateMhs * 1e6);
}

export function MinerPage({ rpcUrl, nodeRunning }: { rpcUrl: string; nodeRunning: boolean }) {
  const [status, setStatus] = useState<MinerStatus | null>(null);
  const [settings, setSettings] = useState<MinerSettings | null>(null);
  const [node, setNode] = useState<NodeInfo | null>(null);
  const [lines, setLines] = useState<string[]>([]);
  const [dirty, setDirty] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [customDevices, setCustomDevices] = useState(false);
  const logBox = useRef<HTMLPreElement>(null);

  useEffect(() => {
    minerBackend.status().then(setStatus);
    minerBackend.logs().then(setLines);
    minerBackend.getSettings().then((s) => {
      setSettings(s);
      setCustomDevices(!DEVICE_PRESETS.some((d) => d.value === s.devices));
    });
    const unStatus = minerBackend.onStatus(setStatus);
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
      unLog.then((f) => f());
      clearInterval(id);
    };
  }, []);

  // Sync state, network hashrate and block reward from the local node.
  useEffect(() => {
    if (!nodeRunning) {
      setNode(null);
      return;
    }
    const rpc = new NodeRpc(rpcUrl);
    let cancelled = false;
    const poll = async () => {
      try {
        await rpc.connect();
        const m = await rpc.metrics();
        const reward = await backend.blockRewardInfo(m.virtualDaaScore);
        if (!cancelled) {
          setNode({
            isSynced: m.isSynced,
            networkHashesPerSecond: m.networkHashesPerSecond,
            blockRewardSompi: reward.currentRewardSompi,
          });
        }
      } catch {
        if (!cancelled) setNode(null);
      }
    };
    void poll();
    const id = setInterval(poll, NODE_POLL_MS);
    return () => {
      cancelled = true;
      clearInterval(id);
      void rpc.disconnect();
    };
  }, [rpcUrl, nodeRunning]);

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

  const active = status.state === "starting" || status.state === "waiting" || status.state === "running" || status.state === "stopping";
  // A scheduled automatic restart is also something the user can stop.
  const stoppable = active || status.restartPending;
  const perBlock = averageSecondsPerBlock(status.hashrateMhs, node?.networkHashesPerSecond ?? null);

  let nodeHint: string | null = null;
  if (!nodeRunning) nodeHint = "The node is not running. Start it on the KARLSEN tab; mining begins once it is fully synced.";
  else if (status.state === "waiting" || (node && !node.isSynced))
    nodeHint = "The node is still syncing. The GPUs stay idle until it is fully synced, then mining begins by itself.";
  else if (status.state === "starting") nodeHint = "The node is synced. The GPUs are being prepared (building the DAG takes about a minute).";

  return (
    <div className="page settings miner-page">
      <div className="node-header">
        <div>
          <div className="node-title">
            Solo Mining · <span className={`state-${status.state}`}>{stateLabel(status)}</span>
          </div>
          <div className="node-sub">
            KarlsenMiner v{status.minerVersion} → your own node · no pool, no stratum bridge, no fee
            {status.minerPid != null ? ` · pid ${status.minerPid}` : ""}
          </div>
        </div>
        <div className="actions">
          {!stoppable && <button className="primary" disabled={busy} onClick={start}>Start mining</button>}
          {stoppable && (
            <button disabled={busy || status.state === "stopping"} onClick={() => run(minerBackend.stop)}>Stop mining</button>
          )}
        </div>
      </div>

      {(error || status.lastError) && <div className="node-error">{error ?? status.lastError}</div>}
      {stoppable && nodeHint && <div className="hint">{nodeHint}</div>}

      <section>
        <div className="tiles">
          <Tile label="Your Hashrate" value={status.hashrateMhs != null ? `${status.hashrateMhs.toFixed(2)} MH/s` : "—"} />
          <Tile label="Network Hashrate" value={node ? formatHashrate(node.networkHashesPerSecond) : "—"} />
          <Tile
            label="Average Time Per Block"
            value={perBlock != null ? `~${formatDuration(Math.round(perBlock))}` : "—"}
            sub="At your current hashrate; luck varies widely"
          />
          <Tile
            label="Block Reward"
            value={node?.blockRewardSompi != null ? `${formatKls(BigInt(node.blockRewardSompi))} KLS` : "—"}
            sub="Paid to your mining address per block found"
          />
          <Tile
            label="Blocks Found"
            value={formatInt(status.blocksFound)}
            sub={status.blocksRejected > 0 ? `${formatInt(status.blocksRejected)} rejected by the node · since Karlsen Desktop started` : "Since Karlsen Desktop started"}
          />
          <Tile label="Mining Uptime" value={status.uptimeSecs != null ? formatUptime(status.uptimeSecs) : "—"} />
        </div>
      </section>

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
          You mine directly on the node inside Karlsen Desktop. Every block you find is submitted by your own node and
          its full reward goes to the mining address; use a receive address from the WALLET tab. The address is checked
          (including its checksum) before mining starts. There are no pool payouts in between: you earn only when you
          find a block, and a single GPU can go a long time without one (see Average Time Per Block). The miner
          talks to the node on this computer only (127.0.0.1).
        </p>
        <label>
          <input type="checkbox" checked={settings.autoStart} onChange={(e) => set("autoStart", e.target.checked)} />
          {" "}Start mining automatically when Karlsen Desktop opens
        </label>
        <div className="actions" style={{ marginTop: 10 }}>
          <button disabled={!dirty || busy} onClick={save}>Save</button>
          {dirty && stoppable && <span className="hint" style={{ marginTop: 0 }}>Changes apply the next time mining starts.</span>}
        </div>
      </section>

      <section>
        <h2>Miner Output</h2>
        <pre ref={logBox} className="log miner-log">
          {lines.length === 0 && <div className="hint">No output yet.</div>}
          {lines.map((l, i) => (
            <div
              key={i}
              className={
                l.includes("ERROR") ? "log-err" : l.startsWith("DESKTOP:") ? "log-desktop" : l.includes("Block accepted") ? "log-block" : ""
              }
            >
              {l}
            </div>
          ))}
        </pre>
      </section>
    </div>
  );
}
