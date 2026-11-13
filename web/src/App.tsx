import { useEffect, useRef, useState } from "react";
import type { ReactNode } from "react";
import { api, getToken, setToken } from "./api";
import Login from "./components/Login";
import Overview from "./components/Overview";
import Opportunities from "./components/Opportunities";
import Pools from "./components/Pools";
import Executions from "./components/Executions";
import StrategyConfig from "./components/StrategyConfig";
import Providers from "./components/Providers";
import RiskControl from "./components/RiskControl";
import Logs from "./components/Logs";
import TelegramPanel from "./components/TelegramPanel";
import type { StatusSnapshot } from "./api";

const SECTIONS: { id: string; label: string; el: (status: StatusSnapshot | null, reload: () => void) => ReactNode }[] = [
  { id: "overview", label: "A · Overview", el: (s) => <Overview status={s} /> },
  { id: "opportunities", label: "B · Live Opportunities", el: () => <Opportunities /> },
  { id: "pools", label: "C · Pools & Liquidity", el: () => <Pools /> },
  { id: "executions", label: "D · Execution Monitor", el: () => <Executions /> },
  { id: "strategy", label: "E · Strategy Config", el: (_s, reload) => <StrategyConfig onSaved={reload} /> },
  { id: "providers", label: "F · Providers", el: () => <Providers /> },
  { id: "risk", label: "G · Risk & Emergency", el: (s, reload) => <RiskControl status={s} onChanged={reload} /> },
  { id: "logs", label: "H · Logs & Analytics", el: () => <Logs /> },
  { id: "telegram", label: "I · Telegram", el: () => <TelegramPanel /> },
];

export default function App() {
  const [authed, setAuthed] = useState<boolean>(!!getToken());
  const [status, setStatus] = useState<StatusSnapshot | null>(null);
  const [section, setSection] = useState("overview");
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

    // Reconnect with capped exponential backoff + jitter so a fleet of tabs
    // never reconnects in lockstep (thundering herd). Bounded attempts; the
    // REST poller keeps the UI fresh regardless. A 401/expired token surfaces
    // through the next REST poll instead of a reload loop.
    let attempt = 0;
    let reconnectTimer: number | null = null;
    let disposed = false;

    const connect = () => {
      if (disposed) return;
      const ws = new WebSocket(api.wsUrl());
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

  const active = SECTIONS.find((s) => s.id === section) ?? SECTIONS[0];

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
      <nav className="tabs">
        {SECTIONS.map((s) => (
          <button
            key={s.id}
            className={`tab ${section === s.id ? "active" : ""}`}
            onClick={() => setSection(s.id)}
          >
            {s.label}
          </button>
        ))}
      </nav>
      <main>{active.el(status, reload)}</main>
    </div>
  );
}
