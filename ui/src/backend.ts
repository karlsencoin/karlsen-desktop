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

export type MinerState = "stopped" | "starting" | "running" | "stopping" | "crashed";

export interface MinerInstallProgress {
  stage: "downloading" | "verifying" | "extracting";
  receivedBytes: number;
  totalBytes: number | null;
}

export interface MinerStatus {
  /** False on platforms KarlsenMiner has no build for. */
  supported: boolean;
  /** KarlsenMiner version this Desktop build installs and runs. */
  version: string;
  installed: boolean;
  install: MinerInstallProgress | null;
  state: MinerState;
  pid: number | null;
  uptimeSecs: number | null;
  /** Total hashrate in MH/s from the miner's latest status line. */
  hashrateMhs: number | null;
  accepted: number | null;
  rejected: number | null;
  restarts: number;
  /** An automatic restart is scheduled after an unexpected exit. */
  restartPending: boolean;
  lastExitCode: number | null;
  lastError: string | null;
}

export interface MinerSettings {
  miningAddress: string;
  workerName: string;
  poolUrl: string;
  /** KarlsenMiner -d value: all | amd | nvidia | intel | amd:0,1 | nvidia:0 | 0,1,3 */
  devices: string;
  /** Pause a GPU at this core temperature (°C); null = miner default. */
  gpuOffTemp: number | null;
  autoStart: boolean;
}

export interface MinerPool {
  name: string;
  payout: string;
  url: string;
}

export const minerBackend = {
  status: () => invoke<MinerStatus>("miner_status"),
  logs: () => invoke<string[]>("miner_logs"),
  pools: () => invoke<MinerPool[]>("miner_pools"),
  install: () => invoke<void>("miner_install"),
  uninstall: () => invoke<void>("miner_uninstall"),
  start: () => invoke<void>("miner_start"),
  stop: () => invoke<void>("miner_stop"),
  getSettings: () => invoke<MinerSettings>("get_miner_settings"),
  saveSettings: (settings: MinerSettings) => invoke<void>("save_miner_settings", { settings }),
  onStatus: (cb: (s: MinerStatus) => void): Promise<UnlistenFn> =>
    listen<MinerStatus>("miner-status", (e) => cb(e.payload)),
  onInstallProgress: (cb: (p: MinerInstallProgress) => void): Promise<UnlistenFn> =>
    listen<MinerInstallProgress>("miner-install", (e) => cb(e.payload)),
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
