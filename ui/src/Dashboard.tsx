import { useEffect, useRef, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { backend, type NodeStatus } from "./backend";
import { NodeRpc, type NodeMetrics } from "./rpc";
import { formatHashrate, formatInt, formatMedianTime, formatUptime } from "./format";

const POLL_MS = 2000;

export const LINKS = [
  { label: "Karlsen Graph Interface (KGI)", url: "https://kgi.karlsencoin.org/" },
  { label: "Block Explorer", url: "https://explorer.karlsencoin.org/" },
  { label: "Web Wallet", url: "https://wallet.karlsencoin.org/" },
  { label: "Website", url: "https://karlsencoin.org/" },
  { label: "Source Code", url: "https://github.com/karlsencoin" },
];

function Tile({ label, value }: { label: string; value: string }) {
  return (
    <div className="tile">
      <div className="tile-label">{label}</div>
      <div className="tile-value">{value}</div>
    </div>
  );
}

function syncLabel(m: NodeMetrics | null): string {
  if (!m) return "WAITING…";
  if (m.isSynced) return "SYNCED";
  const behindSecs = Math.max(0, (Date.now() - Number(m.pastMedianTime)) / 1000);
  const days = behindSecs / 86400;
  return days >= 1 ? `SYNCING (${days.toFixed(1)} d behind)` : `SYNCING (${Math.round(behindSecs / 60)} min behind)`;
}

export function Dashboard({ status, rpcUrl }: { status: NodeStatus | null; rpcUrl: string }) {
  const [metrics, setMetrics] = useState<NodeMetrics | null>(null);
  const [rpcError, setRpcError] = useState<string | null>(null);
  const rpc = useRef<NodeRpc | null>(null);

  useEffect(() => {
    rpc.current = new NodeRpc(rpcUrl);
    let alive = true;
    const tick = async () => {
      if (status?.state !== "running") {
        setMetrics(null);
        return;
      }
      try {
        const m = await rpc.current!.metrics();
        if (alive) {
          setMetrics(m);
          setRpcError(null);
        }
      } catch (e) {
        if (alive) setRpcError(String(e));
      }
    };
    tick();
    const id = setInterval(tick, POLL_MS);
    return () => {
      alive = false;
      clearInterval(id);
      rpc.current?.disconnect();
    };
  }, [rpcUrl, status?.state]);

  const state = status?.state ?? "stopped";
  const running = state === "running" || state === "starting";

  return (
    <div className="page">
      <section>
        <h2>Services</h2>
        <div className="node-header">
          <div>
            <div className="node-title">KARLSEND</div>
            <div className="node-sub">
              {metrics ? `v${metrics.serverVersion}` : "rusty-karlsen"} · <span className={`state state-${state}`}>{state.toUpperCase()}</span>
              {status && status.restarts > 0 && <> · restarts: {status.restarts}</>}
            </div>
          </div>
          <div className="actions">
            {running ? (
              <>
                <button onClick={() => backend.restart()}>Restart</button>
                <button onClick={() => backend.stop()}>Stop</button>
              </>
            ) : (
              <button className="primary" onClick={() => backend.start()}>Start node</button>
            )}
          </div>
        </div>

        <div className="tiles">
          <Tile label="Uptime" value={formatUptime(status?.uptimeSecs ?? null)} />
          <Tile label="Network" value={metrics?.networkName.toUpperCase() ?? "KARLSEN-MAINNET"} />
          <Tile label="Peers" value={metrics ? String(metrics.peers) : "—"} />
          <Tile label="Headers" value={metrics ? formatInt(metrics.headerCount) : "—"} />
          <Tile label="Blocks" value={metrics ? formatInt(metrics.blockCount) : "—"} />
          <Tile label="DAA Score" value={metrics ? formatInt(metrics.virtualDaaScore) : "—"} />
          <Tile label="DAG Sync" value={syncLabel(metrics)} />
          <Tile label="Median Time" value={metrics ? formatMedianTime(metrics.pastMedianTime) : "—"} />
          <Tile label="Network Hashrate" value={metrics ? formatHashrate(metrics.networkHashesPerSecond) : "N/A"} />
          <Tile label="Difficulty" value={metrics ? metrics.difficulty.toExponential(3) : "—"} />
          <Tile label="Mempool" value={metrics ? formatInt(metrics.mempoolSize) : "—"} />
          <Tile label="UTXO Index" value={metrics ? (metrics.hasUtxoIndex ? "ENABLED" : "DISABLED") : "—"} />
        </div>
        {rpcError && state === "running" && <div className="hint">Waiting for node RPC on {rpcUrl}…</div>}
      </section>

      <section>
        <h2>Karlsen Resources</h2>
        <ul className="links">
          {LINKS.map((l) => (
            <li key={l.url}>
              <a href={l.url} onClick={(e) => { e.preventDefault(); openUrl(l.url); }}>{l.label}</a>
            </li>
          ))}
        </ul>
      </section>
    </div>
  );
}
