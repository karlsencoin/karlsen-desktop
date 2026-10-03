import { useEffect, useState } from "react";
import { backend, type NodeStatus, type Settings } from "./backend";
import { Dashboard } from "./Dashboard";
import { LogConsole } from "./LogConsole";
import { SettingsPage } from "./SettingsPage";

type Tab = "karlsen" | "wallet" | "settings" | "karlsend";

const TABS: { id: Tab; label: string }[] = [
  { id: "karlsen", label: "KARLSEN" },
  { id: "wallet", label: "WALLET" },
  { id: "settings", label: "SETTINGS" },
  { id: "karlsend", label: "KARLSEND" },
];

/** Turn a listen address (e.g. 0.0.0.0:43110) into a local ws:// URL. */
function wsUrl(listen: string): string {
  const port = listen.split(":").pop() || "43110";
  return `ws://127.0.0.1:${port}`;
}

export function App() {
  const [tab, setTab] = useState<Tab>("karlsen");
  const [status, setStatus] = useState<NodeStatus | null>(null);
  const [settings, setSettings] = useState<Settings | null>(null);
  // Mount the wallet iframe on first visit and keep it alive afterwards,
  // so switching tabs does not lock the wallet.
  const [walletMounted, setWalletMounted] = useState(false);

  useEffect(() => {
    backend.status().then(setStatus);
    backend.getSettings().then(setSettings);
    const un = backend.onStatus(setStatus);
    // Uptime ticks locally between status events.
    const id = setInterval(() => backend.status().then(setStatus), 1000);
    return () => { un.then((f) => f()); clearInterval(id); };
  }, []);

  useEffect(() => { if (tab === "wallet") setWalletMounted(true); }, [tab]);

  const rpcUrl = wsUrl(settings?.rpcListenBorsh ?? "127.0.0.1:43110");

  return (
    <div className="app">
      <header>
        <div className="brand">Karlsen Desktop <span className="ver">v3.0.0</span></div>
        <nav>
          {TABS.map((t) => (
            <button key={t.id} className={tab === t.id ? "tab active" : "tab"} onClick={() => setTab(t.id)}>{t.label}</button>
          ))}
        </nav>
      </header>
      <main>
        {tab === "karlsen" && <Dashboard status={status} rpcUrl={rpcUrl} />}
        {tab === "settings" && <SettingsPage status={status} onSaved={setSettings} />}
        {tab === "karlsend" && <LogConsole />}
        {walletMounted && (
          <iframe
            title="Karlsen Wallet"
            className="wallet-frame"
            style={{ display: tab === "wallet" ? "block" : "none" }}
            src={`/wallet/index.html?node=${encodeURIComponent(rpcUrl)}`}
          />
        )}
      </main>
    </div>
  );
}
