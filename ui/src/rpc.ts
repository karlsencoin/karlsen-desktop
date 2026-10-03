// Node metrics over wRPC (Borsh) using the Karlsen WASM SDK.
// The SDK is not bundled by Vite: scripts/build-wallet.sh copies it from the
// wallet submodule into public/karlsen-wasm/ so both apps share one SDK build.

export interface NodeMetrics {
  serverVersion: string;
  networkName: string;
  isSynced: boolean;
  hasUtxoIndex: boolean;
  peers: number;
  headerCount: bigint;
  blockCount: bigint;
  virtualDaaScore: bigint;
  difficulty: number;
  pastMedianTime: bigint;
  mempoolSize: bigint;
  networkHashesPerSecond: bigint | null;
}

// eslint-disable-next-line @typescript-eslint/no-explicit-any
type Sdk = any;

let sdkPromise: Promise<Sdk> | null = null;

async function loadSdk(): Promise<Sdk> {
  if (!sdkPromise) {
    sdkPromise = (async () => {
      const url = "/karlsen-wasm/karlsen.js";
      const sdk = await import(/* @vite-ignore */ url);
      await sdk.default({ module_or_path: "/karlsen-wasm/karlsen_bg.wasm" });
      return sdk;
    })();
  }
  return sdkPromise;
}

export class NodeRpc {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  private client: any = null;

  constructor(private url: string) {}

  get connected(): boolean {
    return !!this.client?.isConnected;
  }

  async connect(): Promise<void> {
    const sdk = await loadSdk();
    if (!this.client) {
      this.client = new sdk.RpcClient({
        url: this.url,
        encoding: sdk.Encoding.Borsh,
        networkId: "mainnet",
      });
    }
    if (!this.client.isConnected) {
      await this.client.connect({ blockAsyncConnect: true, timeoutDuration: 3000, retryInterval: 0 });
    }
  }

  async disconnect(): Promise<void> {
    await this.client?.disconnect();
  }

  async metrics(): Promise<NodeMetrics> {
    await this.connect();
    const c = this.client;
    const [server, dag, peers, info] = await Promise.all([
      c.getServerInfo(),
      c.getBlockDagInfo(),
      c.getConnectedPeerInfo(),
      c.getInfo(),
    ]);
    let hashrate: bigint | null = null;
    try {
      // Only meaningful once synced; can fail on a fresh node.
      const r = await c.estimateNetworkHashesPerSecond({ windowSize: 1000 });
      hashrate = BigInt(r.networkHashesPerSecond);
    } catch {
      hashrate = null;
    }
    return {
      serverVersion: server.serverVersion,
      networkName: dag.network ?? dag.networkName ?? "karlsen-mainnet",
      isSynced: server.isSynced,
      hasUtxoIndex: server.hasUtxoIndex,
      peers: peers.peerInfo?.length ?? 0,
      headerCount: BigInt(dag.headerCount),
      blockCount: BigInt(dag.blockCount),
      virtualDaaScore: BigInt(dag.virtualDaaScore),
      difficulty: Number(dag.difficulty),
      pastMedianTime: BigInt(dag.pastMedianTime),
      mempoolSize: BigInt(info.mempoolSize),
      networkHashesPerSecond: hashrate,
    };
  }
}
