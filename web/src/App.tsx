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
    const ws = new WebSocket(api.wsUrl());
    ws.onclose = () => {
      // Reconnect after a delay; the REST poller keeps the UI fresh meanwhile.
      window.setTimeout(() => window.location.reload(), 10_000);
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
    return () => {
      if (timerRef.current) window.clearInterval(timerRef.current);
      ws.close();
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
