import { useEffect, useState } from "react";
import { backend, type NodeStatus, type Settings } from "./backend";
import { Dashboard } from "./Dashboard";
import { LogConsole } from "./LogConsole";
import { SettingsPage } from "./SettingsPage";
import { openUrl } from "@tauri-apps/plugin-opener";

type Tab = "karlsen" | "wallet" | "settings" | "karlsend";

const TABS: { id: Tab; label: string }[] = [
  { id: "karlsen", label: "KARLSEN" },
  { id: "wallet", label: "WALLET" },
  { id: "settings", label: "SETTINGS" },
  { id: "karlsend", label: "KARLSEND" },
];

/**
 * The embedded wallet is same-origin, so its clicks can be observed from here. The Tauri webview
 * does not open new windows, so links that leave the app (explorer, Phantom, legacy wallet) are
 * handed to the system browser instead. Only http(s) URLs on another origin are intercepted.
 */
function routeExternalLinks(frame: HTMLIFrameElement) {
  const doc = frame.contentDocument;
  if (!doc) return;
  doc.addEventListener(
    "click",
    (e) => {
      const a = (e.target as Element | null)?.closest?.("a[href]") as HTMLAnchorElement | null;
      if (!a) return;
      let url: URL;
      try {
        url = new URL(a.href, doc.baseURI);
      } catch {
        return;
      }
      if ((url.protocol === "https:" || url.protocol === "http:") && url.origin !== window.location.origin) {
        e.preventDefault();
        void openUrl(url.toString()).catch((err) => console.warn("openUrl refused", url.toString(), err));
      }
    },
    true,
  );
}

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
        <div className="brand"><img className="brand-logo" src="/karlsen-logo.svg" alt="" />Karlsen Desktop <span className="ver">v3.0.0</span></div>
        <nav>
          {TABS.map((t) => (
            <button key={t.id} className={tab === t.id ? "tab active" : "tab"} onClick={() => setTab(t.id)}>{t.label}</button>
          ))}
        </nav>
      </header>
      <main>
        {tab === "karlsen" && <Dashboard status={status} rpcUrl={rpcUrl} settings={settings} />}
        {tab === "settings" && <SettingsPage status={status} onSaved={setSettings} />}
        {tab === "karlsend" && <LogConsole />}
        {walletMounted && (
          <iframe
            title="Karlsen Wallet"
            className="wallet-frame"
            style={{ display: tab === "wallet" ? "block" : "none" }}
            onLoad={(e) => routeExternalLinks(e.currentTarget)}
            src={`/wallet/index.html?node=${encodeURIComponent(rpcUrl)}`}
          />
        )}
      </main>
    </div>
  );
}
