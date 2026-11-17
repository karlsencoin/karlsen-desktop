// Typed wrappers around the Rust backend commands and events.
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type NodeState = "stopped" | "starting" | "running" | "stopping" | "crashed";

export interface NodeStatus {
  state: NodeState;
  pid: number | null;
  uptimeSecs: number | null;
  restarts: number;
  lastExitCode: number | null;
  /** Unix time (seconds) of the last unexpected exit. */
  lastCrashUnix: number | null;
  /** Why the node is not restarted automatically (e.g. data folder in use). */
  lastError: string | null;
  dataDir: string;
}

export interface Settings {
  dataDir: string;
  autoStartNode: boolean;
  autoRestartNode: boolean;
  rpcListen: string;
  rpcListenBorsh: string;
  p2pListen: string;
  upnpEnabled: boolean;
  utxoIndex: boolean;
  bootstrapPeers: string[];
  runInBackground: boolean;
  extraArgs: string[];
}

/** Mainnet block reward schedule, computed in Rust from the karlsend subsidy table. */
export interface BlockRewardInfo {
  currentRewardSompi: number;
  nextRewardSompi: number;
  nextReductionDaaScore: number | null;
  secondsUntilNextReduction: number | null;
}

export type MinerState = "stopped" | "starting" | "waiting" | "running" | "stopping" | "crashed";

export interface MinerStatus {
  /** Versions of the bundled KarlsenMiner and Karlsen stratum bridge. */
  minerVersion: string;
  bridgeVersion: string;
  /** "waiting" = the bridge runs and waits for the node to be reachable and fully synced. */
  state: MinerState;
  minerPid: number | null;
  bridgePid: number | null;
  /** Seconds since the miner started hashing in the current session. */
  uptimeSecs: number | null;
  /** Total hashrate in MH/s from the miner's latest status line. */
  hashrateMhs: number | null;
  /** Shares only measure the hashrate; rewards come from blocks. */
  accepted: number | null;
  rejected: number | null;
  /** Blocks this node accepted from the miner since Karlsen Desktop started. */
  blocksFound: number;
  restarts: number;
  /** An automatic restart is scheduled after an unexpected exit. */
  restartPending: boolean;
  lastError: string | null;
}

export interface MinerSettings {
  /** Karlsen address that receives the block rewards. */
  miningAddress: string;
  workerName: string;
  /** Loopback TCP port of the bundled stratum bridge. */
  stratumPort: number;
  /** KarlsenMiner -d value: all | amd | nvidia | intel | amd:0,1 | nvidia:0 | 0,1,3 */
  devices: string;
  /** Pause a GPU at this core temperature (°C); null = miner default. */
  gpuOffTemp: number | null;
  autoStart: boolean;
}

export const minerBackend = {
  status: () => invoke<MinerStatus>("miner_status"),
  logs: () => invoke<string[]>("miner_logs"),
  start: () => invoke<void>("miner_start"),
  stop: () => invoke<void>("miner_stop"),
  getSettings: () => invoke<MinerSettings>("get_miner_settings"),
  saveSettings: (settings: MinerSettings) => invoke<void>("save_miner_settings", { settings }),
  onStatus: (cb: (s: MinerStatus) => void): Promise<UnlistenFn> =>
    listen<MinerStatus>("miner-status", (e) => cb(e.payload)),
  onLog: (cb: (line: string) => void): Promise<UnlistenFn> =>
    listen<string>("miner-log", (e) => cb(e.payload)),
};

export const backend = {
  blockRewardInfo: (daaScore: bigint) =>
    invoke<BlockRewardInfo>("block_reward_info", { daaScore: Number(daaScore) }),
  status: () => invoke<NodeStatus>("node_status"),
  logs: () => invoke<string[]>("node_logs"),
  start: () => invoke<void>("node_start"),
  stop: () => invoke<void>("node_stop"),
  restart: () => invoke<void>("node_restart"),
  deleteAndResync: () => invoke<void>("node_delete_and_resync"),
  getSettings: () => invoke<Settings>("get_settings"),
  saveSettings: (settings: Settings) => invoke<void>("save_settings", { settings }),
  onStatus: (cb: (s: NodeStatus) => void): Promise<UnlistenFn> =>
    listen<NodeStatus>("node-status", (e) => cb(e.payload)),
  onLog: (cb: (line: string) => void): Promise<UnlistenFn> =>
    listen<string>("node-log", (e) => cb(e.payload)),
};
