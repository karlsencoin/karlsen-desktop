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
