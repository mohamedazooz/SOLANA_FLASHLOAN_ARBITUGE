import { useEffect, useRef, useState } from "react";
import type { ReactNode } from "react";
import { api, getToken, setToken } from "./api";
import Login from "./components/Login";
import Dashboard from "./components/Dashboard";
import Terminal from "./components/Terminal";
import Overview from "./components/Overview";
import Opportunities from "./components/Opportunities";
import Pools from "./components/Pools";
import Executions from "./components/Executions";
import StrategyConfig from "./components/StrategyConfig";
import Providers from "./components/Providers";
import RouteMonitor from "./components/RouteMonitor";
import RiskControl from "./components/RiskControl";
import Logs from "./components/Logs";
import TelegramPanel from "./components/TelegramPanel";
import type { StatusSnapshot } from "./api";
import { ArabicToggle, ExplanationPanel } from "./components/ArabicTooltips";

const SECTIONS = [
  { id: "dashboard", label: "🏠 Dashboard" },
  { id: "terminal", label: "💻 Terminal" },
  { id: "overview", label: "📊 Overview" },
  { id: "opportunities", label: "🎯 Opportunities" },
  { id: "pools", label: "💧 Pools" },
  { id: "executions", label: "📋 Executions" },
  { id: "strategy", label: "⚙️ Strategy" },
  { id: "providers", label: "📡 Providers" },
  { id: "routes", label: "🌐 Routes" },
  { id: "risk", label: "🛡️ Risk" },
  { id: "logs", label: "📝 Logs" },
  { id: "telegram", label: "📱 Telegram" },
];

export default function App() {
  const [authed, setAuthed] = useState<boolean>(!!getToken());
  const [status, setStatus] = useState<StatusSnapshot | null>(null);
  const [section, setSection] = useState("dashboard");
  const [showArabic, setShowArabic] = useState(true);
  const wsRef = useRef<WebSocket | null>(null);
  const timerRef = useRef<number | null>(null);

  const reload = () => {
    api
      .status()
      .then(setStatus)
      .catch(() => setAuthed(false));
  };

  useEffect(() => {
    if (!authed) return;
    reload();
    timerRef.current = window.setInterval(reload, 5_000);

    let attempt = 0;
    let reconnectTimer: number | null = null;
    let disposed = false;

    const connect = () => {
      if (disposed) return;
      api
        .wsUrl()
        .then((url) => {
          if (disposed) return;
          const ws = new WebSocket(url);
          ws.onopen = () => {
            attempt = 0;
          };
          ws.onclose = () => {
            if (disposed || attempt >= 8) return;
            const backoff = Math.min(500 * 2 ** attempt, 30_000);
            const jitter = Math.random() * 500;
            attempt += 1;
            reconnectTimer = window.setTimeout(connect, backoff + jitter);
          };
          ws.onmessage = (ev) => {
            try {
              const parsed = JSON.parse(ev.data);
              if (parsed && typeof parsed === "object" && "mode" in parsed) {
                setStatus(parsed as StatusSnapshot);
              }
            } catch {
              /* ignore malformed frames */
            }
          };
          wsRef.current = ws;
        })
        .catch(() => {
          if (disposed || attempt >= 8) return;
          const backoff = Math.min(500 * 2 ** attempt, 30_000);
          attempt += 1;
          reconnectTimer = window.setTimeout(connect, backoff);
        });
    };
    connect();

    return () => {
      disposed = true;
      if (reconnectTimer !== null) window.clearTimeout(reconnectTimer);
      if (timerRef.current) window.clearInterval(timerRef.current);
      wsRef.current?.close();
    };
  }, [authed]);

  if (!authed) {
    return (
      <Login
        onLogin={() => {
          setAuthed(true);
        }}
      />
    );
  }

  const active = SECTIONS.find((s) => s.id === section);

  return (
    <div className="app">
      <header className="topbar">
        <div className="brand">
          <span className="logo">⚡</span>
          <div>
            <h1>Solana Flash-Loan Arbitrage</h1>
            <span className="sub">Unified control · V1 transport · Kamino + Meteora DLMM</span>
          </div>
        </div>
        <div className="topbar-status">
          {status && (
            <>
              <span className={`badge mode-${status.mode}`}>{status.mode}</span>
              <span className={`badge ${status.paused ? "badge-red" : "badge-green"}`}>
                {status.paused ? "paused" : "running"}
              </span>
              <span className="mono">slot {status.slot.toLocaleString()}</span>
            </>
          )}
          <ArabicToggle showArabic={showArabic} onToggle={setShowArabic} />
          <button
            className="btn btn-ghost"
            onClick={async () => {
              await api.logout().catch(() => {});
              setToken(null);
              setAuthed(false);
            }}
          >
            Logout
          </button>
        </div>
      </header>
      <nav className="tabs" role="tablist" aria-label="Main navigation">
        {SECTIONS.map((s) => (
          <button
            key={s.id}
            role="tab"
            aria-selected={section === s.id}
            className={`tab ${section === s.id ? "active" : ""}`}
            onClick={() => setSection(s.id)}
          >
            {s.label}
          </button>
        ))}
      </nav>
      <main>
        {section === "dashboard" && <Dashboard />}
        {section === "terminal" && <Terminal />}
        {section === "overview" && <Overview status={status} />}
        {section === "opportunities" && <Opportunities />}
        {section === "pools" && <Pools />}
        {section === "executions" && <Executions />}
        {section === "strategy" && <StrategyConfig onSaved={reload} />}
        {section === "providers" && <Providers />}
        {section === "routes" && <RouteMonitor />}
        {section === "risk" && <RiskControl status={status} onChanged={reload} />}
        {section === "logs" && <Logs />}
        {section === "telegram" && <TelegramPanel />}
        
        {/* Server Info Panel */}
        <div style={{ marginTop: "24px" }}>
          <ExplanationPanel
            title="Server & Deployment"
            titleAr="الخادم والنشر"
            icon="🖥️"
            showArabic={showArabic}
          >
            <p>
              <strong>Bot Server:</strong> The arbitrage bot runs as a systemd service on the VPS.
              <code>arb-bot.service</code> manages the Rust binary with auto-restart on failure.
            </p>
            <p>
              <strong>Start Script:</strong> <code>scripts/deploy_vps.sh</code> deploys the bot to the server.
              <code>scripts/arb-bot.service</code> is the systemd unit file.
            </p>
            <p>
              <strong>Windows Batch:</strong> <code>scripts/start-bot-server.bat</code> starts the bot locally on Windows for testing.
              <code>scripts/run-dashboard.bat</code> starts the web dashboard.
            </p>
            <p>
              <strong>Shadow Mode:</strong> <code>docker-compose.shadow.yml</code> runs the bot in shadow mode
              with simulated execution for safe testing.
            </p>
            <p>
              <strong>Logs:</strong> Bot logs are at <code>/var/log/arb-bot/</code> on the server.
              Use <code>journalctl -u arb-bot -f</code> to follow live logs.
            </p>
          </ExplanationPanel>
        </div>
      </main>
    </div>
  );
}
