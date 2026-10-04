import { useEffect, useRef, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { backend, type BlockRewardInfo, type NodeStatus } from "./backend";
import { NodeRpc, type ChainInfo, type NodeMetrics } from "./rpc";
import {
  formatDifficulty,
  formatDuration,
  formatFeeRate,
  formatFeeTime,
  formatHashrate,
  formatInt,
  formatKls,
  formatLocalDateTime,
  formatMedianTime,
  formatPercent,
  formatUptime,
  formatUtc,
} from "./format";

const POLL_MS = 2000;
/** Supply and fee estimates change slowly; poll them less often. */
const CHAIN_POLL_MS = 10_000;
/** Block detail page on the explorer; the block hash is appended. */
const EXPLORER_BLOCK_URL = "https://explorer.karlsencoin.org/blocks/";
/** Number of DAG tip hashes shown. */
const MAX_TIPS = 6;

export const LINKS = [
  { label: "Karlsen Graph Interface (KGI)", url: "https://kgi.karlsencoin.org/" },
  { label: "Block Explorer", url: "https://explorer.karlsencoin.org/" },
  { label: "Web Wallet", url: "https://wallet.karlsencoin.org/" },
  { label: "Website", url: "https://karlsencoin.org/" },
  { label: "Source Code", url: "https://github.com/karlsencoin" },
];

function Tile({ label, value, title, sub }: { label: string; value: string; title?: string; sub?: string }) {
  return (
    <div className="tile" title={title ?? (sub ? `${value} · ${sub}` : value)}>
      <div className="tile-label">{label}</div>
      <div className="tile-value">{value}</div>
      {sub && <div className="tile-sub">{sub}</div>}
    </div>
  );
}

function open(url: string) {
  return (e: React.MouseEvent) => {
    e.preventDefault();
    openUrl(url);
  };
}

/** "crashed 2× · last 23:01:12 · exit code 1", shown only while a crash streak is active. */
function crashLabel(status: NodeStatus | null): string | null {
  if (!status || status.restarts === 0) return null;
  const parts = [`crashed ${status.restarts}×`];
  if (status.lastCrashUnix != null) parts.push(`last ${formatLocalDateTime(status.lastCrashUnix * 1000).slice(11)}`);
  if (status.lastExitCode != null) parts.push(`exit code ${status.lastExitCode}`);
  return parts.join(" · ");
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
  const [chain, setChain] = useState<ChainInfo | null>(null);
  const [reward, setReward] = useState<BlockRewardInfo | null>(null);
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

  // Supply and fee estimates: separate, slower loop sharing the same RPC client.
  useEffect(() => {
    let alive = true;
    const tick = async () => {
      if (status?.state !== "running" || !rpc.current) {
        setChain(null);
        return;
      }
      try {
        const c = await rpc.current.chainInfo();
        if (alive) setChain(c);
      } catch {
        /* node still starting; the metrics loop reports RPC errors */
      }
    };
    tick();
    const id = setInterval(tick, CHAIN_POLL_MS);
    return () => {
      alive = false;
      clearInterval(id);
    };
  }, [rpcUrl, status?.state]);

  // Block reward depends only on the DAA score; recompute when it moves.
  const daaScore = metrics?.virtualDaaScore;
  useEffect(() => {
    if (daaScore == null) return;
    let alive = true;
    backend
      .blockRewardInfo(daaScore)
      .then((r) => alive && setReward(r))
      .catch(() => alive && setReward(null));
    return () => {
      alive = false;
    };
  }, [daaScore]);

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
              {crashLabel(status) && <> · <span className="state-crashed">{crashLabel(status)}</span></>}
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
          <Tile
            label="Median Time (local)"
            value={metrics ? formatMedianTime(metrics.pastMedianTime) : "—"}
            title={metrics ? formatUtc(Number(metrics.pastMedianTime)) : undefined}
          />
          <Tile label="Network Hashrate" value={metrics ? formatHashrate(metrics.networkHashesPerSecond) : "N/A"} />
          <Tile
            label="Difficulty"
            value={metrics ? formatDifficulty(metrics.difficulty) : "—"}
            title={metrics ? formatInt(Math.round(metrics.difficulty)) : undefined}
          />
          <Tile label="Mempool" value={metrics ? formatInt(metrics.mempoolSize) : "—"} />
          <Tile label="UTXO Index" value={metrics ? (metrics.hasUtxoIndex ? "ENABLED" : "DISABLED") : "—"} />
        </div>
        {rpcError && state === "running" && <div className="hint">Waiting for node RPC on {rpcUrl}… ({rpcError})</div>}
      </section>

      <section>
        <h2>Karlsen Chain</h2>
        <div className="tiles">
          <Tile label="Circulating Supply" value={chain ? formatKls(chain.circulatingSompi, 0) : "—"} />
          <Tile label="Maximum Supply" value={chain ? formatKls(chain.maxSompi, 0) : "—"} />
          <Tile label="Mined (of Maximum Supply)" value={chain ? formatPercent(chain.circulatingSompi, chain.maxSompi) : "—"} />
          <Tile label="Block Reward (per Block)" value={reward ? formatKls(BigInt(reward.currentRewardSompi)) : "—"} />
          <Tile
            label="Next Block Reward Reduction"
            value={
              reward?.secondsUntilNextReduction != null
                ? `in ${formatDuration(reward.secondsUntilNextReduction)}`
                : reward
                  ? "Emission complete"
                  : "—"
            }
            title={
              reward?.secondsUntilNextReduction != null
                ? `${formatLocalDateTime(Date.now() + reward.secondsUntilNextReduction * 1000)} · DAA score ${formatInt(reward.nextReductionDaaScore ?? 0)}`
                : undefined
            }
          />
          <Tile
            label="Block Reward after Reduction"
            value={reward?.nextReductionDaaScore != null ? formatKls(BigInt(reward.nextRewardSompi)) : "—"}
          />
        </div>
        {metrics && !metrics.isSynced && <div className="hint">Supply and rewards reflect the local node and are final once it is synced.</div>}
      </section>

      <section>
        <h2>Transaction Fee Estimates</h2>
        <div className="tiles">
          <Tile label="Priority Fee Rate" value={chain ? formatFeeRate(chain.priorityFee) : "—"} sub={chain ? formatFeeTime(chain.priorityFee) : undefined} />
          <Tile label="Normal Fee Rate" value={chain ? formatFeeRate(chain.normalFee) : "—"} sub={chain ? formatFeeTime(chain.normalFee) : undefined} />
          <Tile label="Low Fee Rate" value={chain ? formatFeeRate(chain.lowFee) : "—"} sub={chain ? formatFeeTime(chain.lowFee) : undefined} />
        </div>
        <div className="help">Fee = fee rate × transaction mass (grams). A simple one-input, two-output transfer is a few thousand grams.</div>
      </section>

      <section>
        <h2>Current DAG Tips</h2>
        {metrics && metrics.tipHashes.length > 0 ? (
          <ul className="tips">
            {metrics.tipHashes.slice(0, MAX_TIPS).map((h) => (
              <li key={h}>
                <a href={EXPLORER_BLOCK_URL + h} onClick={open(EXPLORER_BLOCK_URL + h)} title="Open in Block Explorer">
                  {h}
                </a>
              </li>
            ))}
          </ul>
        ) : (
          <div className="hint">—</div>
        )}
      </section>

      <section>
        <h2>Karlsen Resources</h2>
        <ul className="links">
          {LINKS.map((l) => (
            <li key={l.url}>
              <a href={l.url} onClick={open(l.url)}>{l.label}</a>
            </li>
          ))}
        </ul>
      </section>
    </div>
  );
}
