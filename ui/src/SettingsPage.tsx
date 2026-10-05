import { useEffect, useState } from "react";
import { backend, type NodeStatus, type Settings } from "./backend";

export function SettingsPage({ status, onSaved }: { status: NodeStatus | null; onSaved: (s: Settings) => void }) {
  const [s, setS] = useState<Settings | null>(null);
  const [dirty, setDirty] = useState(false);
  const [msg, setMsg] = useState<string | null>(null);

  useEffect(() => { backend.getSettings().then(setS); }, []);
  if (!s) return <div className="page">Loading…</div>;

  const set = <K extends keyof Settings>(k: K, v: Settings[K]) => { setS({ ...s, [k]: v }); setDirty(true); };

  const save = async (restart: boolean) => {
    await backend.saveSettings(s);
    onSaved(s);
    setDirty(false);
    if (restart) await backend.restart();
    setMsg(restart ? "Saved, node restarted." : "Saved. Node changes apply on next restart.");
  };

  const resync = async () => {
    const ok = window.confirm(
      `This stops the node, deletes the downloaded blockchain data in\n${status?.dataDir || "the data folder"}\nand syncs again from scratch.\n\n` +
        "This is the same folder karlsend uses when started on its own, so a separately run node loses its chain data too.\n\n" +
        "Your wallet is NOT affected. Continue?",
    );
    if (ok) {
      await backend.deleteAndResync();
      setMsg("Chain data deleted, resync started.");
    }
  };

  return (
    <div className="page settings">
      <section>
        <h2>Data Folder</h2>
        <input className="wide" value={s.dataDir} placeholder={status?.dataDir || "default"} onChange={(e) => set("dataDir", e.target.value)} />
        <p className="help">Empty = default location. Currently used: {status?.dataDir || "—"}</p>
      </section>

      <section>
        <h2>Node</h2>
        <label><input type="checkbox" checked={s.autoStartNode} onChange={(e) => set("autoStartNode", e.target.checked)} /> Start node automatically when Karlsen Desktop opens</label>
        <label><input type="checkbox" checked={s.autoRestartNode} onChange={(e) => set("autoRestartNode", e.target.checked)} /> Restart node automatically if it stops unexpectedly</label>
        <label><input type="checkbox" checked={s.utxoIndex} onChange={(e) => set("utxoIndex", e.target.checked)} /> UTXO index (required by the wallet)</label>
        <label><input type="checkbox" checked={s.upnpEnabled} onChange={(e) => set("upnpEnabled", e.target.checked)} /> UPnP: ask your router to open the P2P port so other nodes can reach yours (recommended)</label>
        <p className="help">
          Nodes that accept connections from the internet make the Karlsen network stronger. With UPnP on, your public IP
          address is visible to other nodes and your node uses some upload bandwidth to serve them. Turn it off if you do
          not want that; your node and wallet keep working either way. The KARLSEN tab shows whether other nodes can reach you.
        </p>
        <label><input type="checkbox" checked={s.runInBackground} onChange={(e) => set("runInBackground", e.target.checked)} /> Keep running in the system tray when the window is closed</label>
      </section>

      <section>
        <h2>Network Addresses</h2>
        <div className="grid2">
          <span>gRPC</span><input value={s.rpcListen} onChange={(e) => set("rpcListen", e.target.value)} />
          <span>wRPC (Borsh)</span><input value={s.rpcListenBorsh} onChange={(e) => set("rpcListenBorsh", e.target.value)} />
          <span>P2P</span><input value={s.p2pListen} onChange={(e) => set("p2pListen", e.target.value)} />
          <span>Bootstrap peers</span>
          <input value={s.bootstrapPeers.join(" ")} onChange={(e) => set("bootstrapPeers", e.target.value.split(/[\s,]+/).filter(Boolean))} />
          <span>Extra arguments</span>
          <input value={s.extraArgs.join(" ")} onChange={(e) => set("extraArgs", e.target.value.split(/\s+/).filter(Boolean))} />
        </div>
        <p className="help">Binding RPC to 0.0.0.0 exposes your node's RPC to your network. Keep 127.0.0.1 unless you know you need it.</p>
      </section>

      <div className="actions">
        <button disabled={!dirty} onClick={() => save(false)}>Save</button>
        <button className="primary" disabled={!dirty} onClick={() => save(true)}>Save & restart node</button>
        {msg && <span className="hint">{msg}</span>}
      </div>

      <section>
        <h2>Maintenance</h2>
        <button className="danger" onClick={resync}>Delete chain data and resync</button>
      </section>
    </div>
  );
}
