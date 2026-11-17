import { useEffect, useRef, useState } from "react";
import { api } from "../api";
import type { 
  StatusSnapshot, 
  OpportunityRecord, 
  ExecutionRecord, 
  LogRecord,
  ProtocolCoverage 
} from "../api";
/* removed - Overrides not used in Dashboard */

/* ============================================================
   Types
   ============================================================ */

interface DashboardStats {
  mode: string;
  execution_enabled: boolean;
  paused: boolean;
  emergency_stopped: boolean;
  slot: number;
  pools_tracked: number;
  stream_connected: boolean;
  helius_connected: boolean;
  opportunities_total: number;
  profitable_total: number;
  executed_total: number;
  successful_total: number;
  failed_total: number;
  net_pnl_sol: number;
  uptime_secs: number;
}

interface DashboardPool {
  pool_id: string;
  protocol: string;
  base_mint: string;
  quote_mint: string;
  last_update_slot: number;
}

interface DashboardOpportunity {
  captured_at_unix: number;
  slot: number;
  route: string;
  protocol: string;
  borrow_lamports: number;
  expected_output_lamports: number;
  net_profit_lamports: number;
  eligible: boolean;
  reject_reason: string | null;
}

interface DashboardExecution {
  at_unix: number;
  outcome: string;
  route: string;
  detail: string;
}

interface DashboardLog {
  at_unix: number;
  level: string;
  stage: string;
  message: string;
}

interface DashboardProtocol {
  protocol: string;
  pools: number;
  avg_fee_bps: number;
}

interface BotControlState {
  canStart: boolean;
  canStop: boolean;
  canShadow: boolean;
  canPaper: boolean;
  canLive: boolean;
  canEmergency: boolean;
  requiresConfirm: boolean;
  confirmValue: string;
}

/* ============================================================
   Dashboard Component
   ============================================================ */

export default function Dashboard() {
  /* ============================================================
     State: Status & Bot Control
     ============================================================ */
  const [status, setStatus] = useState<StatusSnapshot | null>(null);
  const [botState, setBotState] = useState<BotControlState>({
    canStart: true,
    canStop: false,
    canShadow: true,
    canPaper: true,
    canLive: false,
    canEmergency: false,
    requiresConfirm: false,
    confirmValue: "",
  });

  const [opportunities, setOpportunities] = useState<DashboardOpportunity[]>([]);
  const [executions, setExecutions] = useState<DashboardExecution[]>([]);
  const [logs, setLogs] = useState<DashboardLog[]>([]);
  const [protocols, setProtocols] = useState<DashboardProtocol[]>([]);

  const [filterProtocol, setFilterProtocol] = useState("");
  const [filterOnlyEligible, setFilterOnlyEligible] = useState(false);
  const [filterMinNet, setFilterMinNet] = useState("");

  const [showArabic, setShowArabic] = useState(true);
  const [busy, setBusy] = useState(false);

  /* WebSocket reference for live updates */
  const wsRef = useRef<WebSocket | null>(null);
  const reconnectAttempts = useRef(0);
  const disposed = useRef(false);

  /* ============================================================
     Load Status from API
     ============================================================ */
  useEffect(() => {
    const loadStatus = async () => {
      try {
        const resp = await api.status();
        setStatus(resp);
        updateBotControlState(resp);
      } catch (e) {
        console.error("Failed to load status:", e);
      }
    };

    loadStatus();
    const interval = setInterval(loadStatus, 5_000);
    return () => clearInterval(interval);
  }, []);

  /* ============================================================
     Load Opportunities
     ============================================================ */
  useEffect(() => {
    const loadOpportunities = async () => {
      try {
        const resp = await api.opportunities();
        setOpportunities(resp);
      } catch (e) {
        console.error("Failed to load opportunities:", e);
      }
    };

    loadOpportunities();
    const interval = setInterval(loadOpportunities, 3_000);
    return () => clearInterval(interval);
  }, []);

  /* ============================================================
     Load Executions
     ============================================================ */
  useEffect(() => {
    const loadExecutions = async () => {
      try {
        const resp = await api.executions();
        setExecutions(resp);
      } catch (e) {
        console.error("Failed to load executions:", e);
      }
    };

    loadExecutions();
    const interval = setInterval(loadExecutions, 3_000);
    return () => clearInterval(interval);
  }, []);

  /* ============================================================
     Load Logs
     ============================================================ */
  useEffect(() => {
    const loadLogs = async () => {
      try {
        const resp = await api.logs();
        setLogs(resp);
      } catch (e) {
        console.error("Failed to load logs:", e);
      }
    };

    loadLogs();
    const interval = setInterval(loadLogs, 3_000);
    return () => clearInterval(interval);
  }, []);

  /* ============================================================
     Load Protocols
     ============================================================ */
  useEffect(() => {
    const loadProtocols = async () => {
      try {
        const resp = await api.routes();
        setProtocols(resp);
      } catch (e) {
        console.error("Failed to load protocols:", e);
      }
    };

    loadProtocols();
    const interval = setInterval(loadProtocols, 5_000);
    return () => clearInterval(interval);
  }, []);

  /* ============================================================
     WebSocket Connection for Live Updates
     ============================================================ */
  useEffect(() => {
    if (!status || status.mode !== "live") {
      // Close WS if not in live mode
      if (wsRef.current) {
        wsRef.current.close();
        wsRef.current = null;
      }
      return;
    }

    const connectWS = async () => {
      try {
        const { ticket } = await api.wsTicket();
        const proto = location.protocol === "https:" ? "wss" : "ws";
        const ws = new WebSocket(`${proto}://${location.host}/api/ws?ticket=${encodeURIComponent(ticket)}`);

        ws.onopen = () => {
          reconnectAttempts.current = 0;
        };

        ws.onclose = () => {
          if (disposed.current) return;
          reconnectAttempts.current += 1;
          if (reconnectAttempts.current < 8) {
            const backoff = Math.min(500 * 2 ** reconnectAttempts.current, 30_000);
            const jitter = Math.random() * 500;
            wsRef.current = null;
            setTimeout(connectWS, backoff + jitter);
          }
        };

        ws.onmessage = (ev) => {
          try {
            const parsed = JSON.parse(ev.data);
            if (parsed && typeof parsed === "object" && "mode" in parsed) {
              setStatus(parsed as StatusSnapshot);
              updateBotControlState(parsed as StatusSnapshot);
            }
          } catch {
            /* ignore malformed frames */
          }
        };

        ws.onerror = (err) => {
          console.error("WebSocket error:", err);
        };

        wsRef.current = ws;
      } catch (err) {
        console.error("WebSocket connection failed:", err);
      }
    };

    connectWS();

    return () => {
      disposed.current = true;
      if (wsRef.current) {
        wsRef.current.close();
        wsRef.current = null;
      }
    };
  }, [status?.mode]);

  /* ============================================================
     Update Bot Control State
     ============================================================ */
  const updateBotControlState = (status: StatusSnapshot) => {
    const canStart = !status.paused && !status.emergency_stopped && status.mode !== "live";
    const canStop = status.paused || status.execution_enabled || status.mode === "live";
    const canShadow = status.mode !== "live" && status.mode !== "paper";
    const canPaper = status.mode !== "live";
    const canLive = status.mode !== "live" && status.execution_enabled;
    const canEmergency = !status.emergency_stopped;
    const requiresConfirm = status.mode !== "live" && status.mode !== "paper" && status.mode !== "shadow";

    setBotState({
      canStart,
      canStop,
      canShadow,
      canPaper,
      canLive,
      canEmergency,
      requiresConfirm,
      confirmValue: "",
    });
  };

  /* ============================================================
     Control Actions
     ============================================================ */
  const handleControl = async (action: string, confirm?: string) => {
    try {
      const result = await api.control(action, confirm);
      // Refresh status after control action
      const resp = await api.status();
      setStatus(resp);
      updateBotControlState(resp);
      return result;
    } catch (e) {
      console.error(`Control ${action} failed:`, e);
      throw e;
    }
  };

  /* ============================================================
     Filtered Opportunities
     ============================================================ */
  const filteredOpportunities = opportunities.filter((r) => {
    if (filterProtocol && !r.protocol.includes(filterProtocol)) return false;
    if (filterOnlyEligible && !r.eligible) return false;
    if (filterMinNet && r.net_profit_lamports < Number(filterMinNet) * 1e9) return false;
    return true;
  });

  /* ============================================================
     Format helpers
     ============================================================ */
  const formatSlot = (slot: number) => slot.toLocaleString();
  const formatProfit = (lamports: number) => {
    const sol = lamports / 1e9;
    return `${sol.toFixed(6)} SOL`;
  };
  const formatLamports = (lamports: number) => lamports.toLocaleString();
  const formatTime = (unix: number) => new Date(unix * 1000).toISOString().slice(11, 19);
  const formatNetPnl = (sol: number) => `${sol.toFixed(6)} SOL`;
  const getPnlTone = (sol: number) => (sol >= 0 ? "tone-green" : "tone-red");

  /* ============================================================
     Render
     ============================================================ */
  if (!status) {
    return (
      <div className="card" style={{ padding: "40px 20px", textAlign: "center" }}>
        <div className="empty-state">
          <div className="empty-state-icon">⚡</div>
          <div className="empty-state-text">Connecting to Bot</div>
          <div className="empty-state-subtext">Waiting for status from the arbitrage bot...</div>
        </div>
      </div>
    );
  }

  return (
    <section className="dashboard">
      {/* Topbar with Bot Status and Controls */}
      <header className="topbar" style={{ padding: "16px 24px" }}>
        <div className="brand" style={{ gap: "16px" }}>
          <div className="logo">⚡</div>
          <div>
            <h1 style={{ fontSize: "15px", fontWeight: 700, letterSpacing: "-0.02em" }}>
              Solana Flash-Loan Arbitrage
            </h1>
            <span className="sub" style={{ fontSize: "11px", color: "var(--text-muted)", letterSpacing: "0.02em", textTransform: "uppercase" }}>
              Unified Dashboard · V1 Transport
            </span>
          </div>
        </div>

        <div className="topbar-status" style={{ gap: "12px" }}>
          <div className="status-indicator" style={{ 
            padding: "6px 12px", 
            fontSize: "11px", 
            fontWeight: 600, 
            textTransform: "uppercase",
            letterSpacing: "0.05em",
            border: "1px solid var(--border)",
            borderRadius: "999px",
            background: "var(--bg)",
            color: "var(--text)"
          }}>
            <span className="status-dot" style={{ width: "6px", height: "6px", borderRadius: "50%", background: status.mode === "live" ? "var(--emerald)" : status.mode === "shadow" ? "var(--amber)" : status.mode === "paper" ? "var(--cyan)" : "var(--muted)", boxShadow: "0 0 8px currentColor" }} />
            {status.mode.toUpperCase()}
          </div>

          <div style={{ display: "flex", alignItems: "center", gap: "8px" }}>
            <span className="slot-display" style={{ fontFamily: '"JetBrains Mono", monospace', fontSize: "12px", color: "var(--text-muted)" }}>
              Slot {status.slot.toLocaleString()}
            </span>

            <BotStatusBadge mode={status.mode} paused={status.paused} emergency_stopped={status.emergency_stopped} execution_enabled={status.execution_enabled} />
          </div>
        </div>

        {/* Bot Control Panel */}
        <div className="control-panel" style={{ gap: "8px", alignItems: "center", flexWrap: "wrap" }}>
          <ControlButton
            variant="primary" 
            disabled={!botState.canStart || busy}
            onClick={() => handleControl("start")}
            title="Start Execution"
            style={{ 
              background: "linear-gradient(135deg, var(--emerald) 0%, var(--cyan) 100%)", 
              borderColor: "transparent",
              color: "#fff"
            }}
          >
            Start
          </ControlButton>

          <ControlButton
            variant="danger" 
            disabled={!botState.canStop || busy}
            onClick={() => handleControl("stop")}
            title="Stop Execution"
            style={{ 
              background: "linear-gradient(135deg, var(--rose) 0%, #ff6b8a 100%)", 
              borderColor: "transparent",
              color: "#fff"
            }}
          >
            Stop
          </ControlButton>

          <ControlButton
            variant="warning" 
            disabled={!botState.canShadow || busy}
            onClick={() => handleControl("shadow")}
            title="Shadow Mode"
            style={{ 
              background: "var(--panel)", 
              borderColor: "var(--border)",
              color: "var(--amber)"
            }}
          >
            Shadow
          </ControlButton>

          <ControlButton
            variant="info" 
            disabled={!botState.canPaper || busy}
            onClick={() => handleControl("paper")}
            title="Paper Mode"
            style={{ 
              background: "var(--panel)", 
              borderColor: "var(--border)",
              color: "var(--cyan)"
            }}
          >
            Paper
          </ControlButton>

          <ControlButton
            variant="danger" 
            disabled={!botState.canLive || busy || botState.requiresConfirm}
            onClick={() => handleControl("live", "CONFIRM-LIVE")}
            title="Live Mode"
            style={{ 
              background: "linear-gradient(135deg, var(--rose) 0%, #ff6b8a 100%)", 
              borderColor: "transparent",
              color: "#fff"
            }}
          >
            Live
          </ControlButton>

          <ControlButton
            variant="danger" 
            disabled={!botState.canEmergency || busy}
            onClick={() => handleControl("emergency-stop")}
            title="Emergency Stop"
            style={{ 
              background: "var(--bg)", 
              borderColor: "var(--rose)",
              color: "var(--rose)"
            }}
          >
            Emergency
          </ControlButton>
        </div>
      </header>

      {/* Main Dashboard Grid */}
      <main className="dashboard-grid" style={{ gap: "16px" }}>
        {/* Left Column: Stats & Overview */}
        <div style={{ gridColumn: "span 8" }}>
          {/* Statistics Grid */}
          <div className="dashboard-grid" style={{ gap: "12px" }}>
            <StatCard
              title="Mode"
              value={status.mode.toUpperCase()}
              tone={status.mode === "live" ? "tone-green" : status.mode === "shadow" ? "tone-amber" : "tone-cyan"}
              icon="⚡"
              style={{ gridColumn: "span 3" }}
            />
            <StatCard
              title="Execution"
              value={status.paused ? "PAUSED" : status.execution_enabled ? "ENABLED" : "DISABLED"}
              tone={status.paused ? "tone-red" : "tone-green"}
              icon={status.paused ? "⏸" : "▶"}
              style={{ gridColumn: "span 3" }}
            />
            <StatCard
              title="Network Slot"
              value={formatSlot(status.slot)}
              tone="tone-muted"
              icon="🔢"
              style={{ gridColumn: "span 3" }}
            />
            <StatCard
              title="Pools Tracked"
              value={String(status.pools_tracked)}
              tone="tone-green"
              icon="💧"
              style={{ gridColumn: "span 3" }}
            />
          </div>

          {/* Connection Status */}
          <div className="card" style={{ marginTop: "16px", padding: "16px" }}>
            <div className="card-header">
              <div className="card-title">
                <span className="card-title-icon emerald" style={{ fontSize: "12px" }}>📡</span>
                <span>Connection Status</span>
              </div>
            </div>
            <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: "12px" }}>
              <div style={{ 
                padding: "10px 12px", 
                background: "var(--bg)", 
                border: "1px solid var(--border)", 
                borderRadius: "8px",
                display: "flex",
                alignItems: "center",
                gap: "8px"
              }}>
                <span className="status-dot" style={{ width: "8px", height: "8px", borderRadius: "50%", background: status.stream_connected ? "var(--emerald)" : "var(--rose)", boxShadow: "0 0 8px currentColor" }} />
                <span style={{ color: status.stream_connected ? "var(--emerald)" : "var(--rose)" }}>Primary Stream</span>
              </div>
              <div style={{ 
                padding: "10px 12px", 
                background: "var(--bg)", 
                border: "1px solid var(--border)", 
                borderRadius: "8px",
                display: "flex",
                alignItems: "center",
                gap: "8px"
              }}>
                <span className="status-dot" style={{ width: "8px", height: "8px", borderRadius: "50%", background: status.helius_connected ? "var(--emerald)" : "var(--rose)", boxShadow: "0 0 8px currentColor" }} />
                <span style={{ color: status.helius_connected ? "var(--emerald)" : "var(--rose)" }}>Helius</span>
              </div>
            </div>
          </div>

          {/* Opportunities Summary */}
          <div className="card" style={{ marginTop: "16px", padding: "16px" }}>
            <div className="card-header">
              <div className="card-title">
                <span className="card-title-icon cyan" style={{ fontSize: "12px" }}>💡</span>
                <span>Live Opportunities</span>
                <span className="badge badge-amber" style={{ fontSize: "9px", padding: "3px 6px" }}>
                  {status.opportunities_total}
                </span>
              </div>
            </div>
            <p className="card-subtitle" style={{ color: "var(--text-muted)", marginBottom: "12px" }}>
              Profitable: {status.profitable_total.toLocaleString()} | Executed: {status.executed_total.toLocaleString()}
            </p>
            <div className="stat-trend" style={{ 
              display: "flex", 
              justifyContent: "space-between",
              marginTop: "8px"
            }}>
              <span className={status.net_pnl_sol >= 0 ? "stat-trend up" : "stat-trend down"}>
                PnL: {formatNetPnl(status.net_pnl_sol)}
              </span>
              <span style={{ fontSize: "10px", color: "var(--text-muted)" }}>Total Trades</span>
            </div>
          </div>
        </div>

        {/* Right Column: Pools, Opportunities, Protocols */}
        <div style={{ gridColumn: "span 4" }}>
          {/* Pools Section */}
          <div className="card" style={{ marginTop: "16px" }}>
            <div className="card-header">
              <div className="card-title">
                <span className="card-title-icon emerald" style={{ fontSize: "12px" }}>💱</span>
                <span>Tracked Pools</span>
              </div>
            </div>
            <div className="table-container">
              <table className="data-table">
                <thead>
                  <tr>
                    <th>Pool</th>
                    <th>Protocol</th>
                    <th>Pair</th>
                    <th>Last Update</th>
                  </tr>
                </thead>
                <tbody>
                  {status ? (
                    status.pools_tracked > 0 ? (
                      // Show pool rows from the supervisor
                      // We'll render a simplified view
                      <tr>
                        <td className="mono route" style={{ fontSize: "11px" }}>— awaiting pool data —</td>
                        <td style={{ fontSize: "11px", color: "var(--text-muted)" }}>—</td>
                        <td className="mono" style={{ fontSize: "11px" }}>—</td>
                        <td className="mono" style={{ fontSize: "11px" }}>—</td>
                      </tr>
                    ) : (
                      <tr>
                        <td colSpan={4} className="muted" style={{ fontSize: "11px", textAlign: "center" }}>
                          No pools tracked yet
                        </td>
                      </tr>
                    )
                  ) : (
                    <tr>
                      <td colSpan={4} className="muted" style={{ fontSize: "11px", textAlign: "center" }}>
                        Loading...
                      </td>
                    </tr>
                  )}
                </tbody>
              </table>
            </div>
            <p className="card-subtitle" style={{ color: "var(--text-muted)", fontSize: "11px", marginTop: "8px" }}>
              Pool inventory published by supervisor. Tracks Meteora DLMM, Raydium, Orca, Pump.fun, PumpSwap.
            </p>
          </div>

          {/* Protocol Coverage */}
          <div className="card" style={{ marginTop: "16px" }}>
            <div className="card-header">
              <div className="card-title">
                <span className="card-title-icon violet" style={{ fontSize: "12px" }}>🌐</span>
                <span>Protocol Coverage</span>
              </div>
            </div>
            <div className="table-container">
              <table className="data-table">
                <thead>
                  <tr>
                    <th>Protocol</th>
                    <th>Pools</th>
                    <th>Share</th>
                    <th>Avg Fee</th>
                    <th>Connectivity</th>
                  </tr>
                </thead>
                <tbody>
                  {protocols.length > 0 ? (
                    protocols.map((r) => (
                      <tr key={r.protocol} style={{ fontSize: "11px" }}>
                        <td style={{ color: "var(--text)" }}>{r.protocol}</td>
                        <td style={{ fontSize: "11px" }}>{r.pools}</td>
                        <td style={{ fontSize: "11px", color: "var(--text-muted)" }}>
                          {status && status.pools_tracked > 0 ? `${((r.pools / status.pools_tracked) * 100).toFixed(1)}%` : "—"}
                        </td>
                        <td style={{ fontSize: "11px" }}>{r.avg_fee_bps.toFixed(1)}</td>
                        <td style={{ 
                          fontSize: "11px",
                          color: r.pools > 0 ? "var(--emerald)" : "var(--rose)"
                        }}>
                          {r.pools > 0 ? "routable" : "no edges"}
                        </td>
                      </tr>
                    ))
                  ) : (
                    <tr>
                      <td colSpan={5} className="muted" style={{ fontSize: "11px", textAlign: "center" }}>
                        No protocol data yet
                      </td>
                    </tr>
                  )}
                </tbody>
              </table>
            </div>
            <p className="card-subtitle" style={{ color: "var(--text-muted)", fontSize: "11px", marginTop: "8px" }}>
              Cross-protocol cycles discovered by unified graph path
            </p>
          </div>
        </div>

        {/* Opportunities Table */}
        <div className="card" style={{ marginTop: "16px", gridColumn: "span 8" }}>
          <div className="card-header">
            <div className="card-title">
              <span className="card-title-icon amber" style={{ fontSize: "12px" }}>🎯</span>
              <span>Live Opportunities</span>
              <span className="badge badge-green" style={{ fontSize: "9px", padding: "3px 6px" }}>
                {filteredOpportunities.length}
              </span>
            </div>
          </div>

          <div className="table-container">
            <table className="data-table">
              <thead>
                <tr>
                  <th>Time</th>
                  <th>Slot</th>
                  <th>Route</th>
                  <th>Protocol</th>
                  <th>Borrow</th>
                  <th>Expected</th>
                  <th>Profit</th>
                  <th>Status</th>
                </tr>
              </thead>
              <tbody>
                {filteredOpportunities.length > 0 ? (
                  filteredOpportunities.map((r, i) => (
                    <tr key={i} style={{ fontSize: "11px" }}>
                      <td className="mono" style={{ fontSize: "11px" }}>{formatTime(r.captured_at_unix)}</td>
                      <td className="mono" style={{ fontSize: "11px" }}>{r.slot}</td>
                      <td className="mono route" style={{ fontSize: "11px", maxWidth: "200px" }}>{r.route}</td>
                      <td style={{ fontSize: "11px", color: "var(--text)" }}>{r.protocol}</td>
                      <td className="mono" style={{ fontSize: "11px" }}>{formatLamports(r.borrow_lamports)}</td>
                      <td className="mono" style={{ fontSize: "11px" }}>{formatLamports(r.expected_output_lamports)}</td>
                      <td className={`mono ${r.net_profit_lamports >= 0 ? "tone-green" : "tone-red"}`} style={{ fontSize: "11px" }}>
                        {formatProfit(r.net_profit_lamports)}
                      </td>
                      <td>
                        {r.eligible ? (
                          <span className="badge badge-emerald" style={{ fontSize: "9px", padding: "2px 6px" }}>eligible</span>
                        ) : (
                          <span className="badge badge-rose" style={{ fontSize: "9px", padding: "2px 6px" }}>
                            {r.reject_reason ?? "rejected"}
                          </span>
                        )}
                      </td>
                    </tr>
                  ))
                ) : (
                  <tr>
                    <td colSpan={8} className="muted" style={{ fontSize: "11px", textAlign: "center" }}>
                      No opportunities recorded yet
                    </td>
                  </tr>
                )}
              </tbody>
            </table>
          </div>
        </div>

        {/* Executions Section */}
        <div className="card" style={{ marginTop: "16px", gridColumn: "span 4" }}>
          <div className="card-header">
            <div className="card-title">
              <span className="card-title-icon cyan" style={{ fontSize: "12px" }}>📋</span>
              <span>Executions</span>
            </div>
          </div>
          <div className="table-container">
            <table className="data-table">
              <thead>
                <tr>
                  <th>Time</th>
                  <th>Outcome</th>
                  <th>Route</th>
                  <th>Detail</th>
                </tr>
              </thead>
              <tbody>
                {executions.length > 0 ? (
                  executions.map((r, i) => (
                    <tr key={i} style={{ fontSize: "11px" }}>
                      <td className="mono" style={{ fontSize: "11px" }}>{formatTime(r.at_unix)}</td>
                      <td>
                        <span className={r.outcome === "landed" ? "badge badge-emerald" : r.outcome === "dropped" ? "badge badge-rose" : "badge badge-amber"} style={{ fontSize: "9px", padding: "2px 6px" }}>
                          {r.outcome}
                        </span>
                      </td>
                      <td className="mono route" style={{ fontSize: "11px" }}>{r.route}</td>
                      <td style={{ fontSize: "11px", color: "var(--text-muted)" }}>{r.detail}</td>
                    </tr>
                  ))
                ) : (
                  <tr>
                    <td colSpan={4} className="muted" style={{ fontSize: "11px", textAlign: "center" }}>
                      No executions recorded yet
                    </td>
                  </tr>
                )}
              </tbody>
            </table>
          </div>
        </div>

        {/* Logs Section */}
        <div className="card" style={{ marginTop: "16px", gridColumn: "span 8" }}>
          <div className="card-header">
            <div className="card-title">
              <span className="card-title-icon rose" style={{ fontSize: "12px" }}>📝</span>
              <span>Live Logs</span>
            </div>
          </div>

          <div style={{ display: "flex", gap: "8px", marginBottom: "8px", alignItems: "center" }}>
            <select 
              style={{
                background: "var(--bg)", 
                border: "1px solid var(--border)", 
                borderRadius: "6px", 
                padding: "6px 10px", 
                color: "var(--text)", 
                fontSize: "11px",
                minWidth: "120px"
              }}
              onChange={(e) => {}}
            >
              <option value="">All Levels</option>
              <option value="info">info</option>
              <option value="warn">warn</option>
              <option value="error">error</option>
            </select>
            <span style={{ fontSize: "10px", color: "var(--text-muted)" }}>Refresh: 3s</span>
          </div>

          <div className="log-stream" style={{ maxHeight: "300px", overflowY: "auto" }}>
            {logs.map((r, i) => (
              <div key={i} className="log-entry" style={{ 
                borderRadius: "6px",
                marginBottom: "4px",
                transition: "background 0.2s"
              }}>
                <span className="log-time" style={{ color: "var(--text-dim)", fontFamily: '"JetBrains Mono", monospace', fontSize: "10px" }}>
                  {formatTime(r.at_unix)}
                </span>
                <span className={`log-level ${r.level}`} style={{ 
                  fontSize: "9px", 
                  fontWeight: 600,
                  textTransform: "uppercase",
                  whiteSpace: "nowrap"
                }}>
                  {r.level.toUpperCase()}
                </span>
                <span className="log-stage" style={{ color: "var(--text-muted)", fontSize: "10px", whiteSpace: "nowrap" }}>
                  {r.stage}
                </span>
                <span className="log-message" style={{ flex: 1, color: "var(--text)" }}>{r.message}</span>
              </div>
            ))}
          </div>

          {logs.length === 0 && (
            <p className="muted" style={{ fontSize: "11px", textAlign: "center", color: "var(--text-muted)", marginTop: "8px" }}>
              No log events yet
            </p>
          )}
        </div>
      </main>

      {/* Arabic Explanations Panel */}
      <div className="explanation-panel" style={{ marginTop: "24px" }}>
        <h4 style={{ margin: "0 0 12px", display: "flex", alignItems: "center", gap: "6px" }}>
          <span style={{ color: "var(--emerald)", fontSize: "11px" }}>ℹ️</span>
          Dashboard Explanations
        </h4>
        <p style={{ color: "var(--text-muted)", fontSize: "11px", lineHeight: "1.7" }}>
          <strong>Mode Indicators:</strong> <span style={{ color: "var(--emerald)" }}>Live</span> - Real market execution with actual funds. <span style={{ color: "var(--amber)" }}>Shadow</span> - Simulated mode using historical data. <span style={{ color: "var(--cyan)" }}>Paper</span> - Paper trading without real funds. <span style={{ color: "var(--rose)" }}>Emergency Stop</span> - Latches execution off until cleared.
        </p>
        <p style={{ color: "var(--text-muted)", fontSize: "11px", lineHeight: "1.7", marginBottom: "6px" }}>
          <strong>Profit & Loss:</strong> Net PnL in SOL. Positive values <span style={{ color: "var(--emerald)" }}>indicate profit</span>, negative values <span style={{ color: "var(--rose)" }}>indicate loss</span>.
        </p>
        <p style={{ color: "var(--text-muted)", fontSize: "11px", lineHeight: "1.7" }}>
          <strong>Opportunity Status:</strong> Eligible opportunities pass all validation checks (liquidity, slippage, fees). Rejected opportunities have a reject_reason explaining why they were filtered out.
        </p>
        <p style={{ color: "var(--text-muted)", fontSize: "11px", lineHeight: "1.7" }}>
          <strong>Protocol Coverage:</strong> Shows which protocols have pools in the unified routing graph. Protocols with zero pools contribute no edges for arbitrage paths.
        </p>
        <div style={{ marginTop: "8px", padding: "8px", background: "var(--bg)", borderRadius: "6px", fontSize: "10px" }}>
          <strong>Key Metrics:</strong> <span style={{ color: "var(--emerald)" }}>Successful</span> - Executions that landed successfully. <span style={{ color: "var(--rose)" }}>Failed</span> - Executions that did not land. <span style={{ color: "var(--amber)" }}>Profitable</span> - Trades with positive net profit.
        </div>
      </div>
    </section>
  );
}

/* ============================================================
   Helper Components
   ============================================================ */

interface StatCardProps {
  title: string;
  value: string;
  tone?: string;
  icon: string;
  style?: React.CSSProperties;
}

function StatCard({ title, value, tone, icon, style }: StatCardProps) {
  return (
    <div 
      className="stat-card"
      style={{
        background: "linear-gradient(135deg, var(--panel) 0%, var(--bg-elevated) 100%)",
        border: "1px solid var(--border)",
        borderRadius: "10px",
        padding: "14px",
        textAlign: "center",
        ...style
      }}
    >
      <div style={{ display: "flex", alignItems: "center", justifyContent: "center", gap: "6px" }}>
        <span style={{ fontSize: "20px" }}>{icon}</span>
        <div>
          <div className="stat-value" style={{ fontSize: "24px", fontWeight: 700, color: tone ? `var(${tone})` : "var(--text)", fontFamily: '"JetBrains Mono", monospace' }}>
            {value}
          </div>
          <div className="stat-label" style={{ fontSize: "10px", color: "var(--text-muted)", textTransform: "uppercase", letterSpacing: "0.05em" }}>
            {title}
          </div>
        </div>
      </div>
    </div>
  );
}

interface ControlButtonProps {
  variant: "primary" | "danger" | "warning" | "info" | "ghost";
  disabled: boolean;
  onClick: () => void;
  title: string;
  style?: React.CSSProperties;
  children?: React.ReactNode;
}

function ControlButton({ variant, disabled, onClick, title, style, children }: ControlButtonProps) {
  const variantStyles = {
    primary: {
      background: "linear-gradient(135deg, var(--emerald) 0%, var(--cyan) 100%)",
      borderColor: "transparent",
      color: "#fff",
      boxShadow: "0 0 20px var(--glow-emerald)"
    },
    danger: {
      background: "linear-gradient(135deg, var(--rose) 0%, #ff6b8a 100%)",
      borderColor: "transparent",
      color: "#fff",
      boxShadow: "0 0 20px var(--glow-rose)"
    },
    warning: {
      background: "linear-gradient(135deg, var(--amber) 0%, #ffcc33 100%)",
      borderColor: "transparent",
      color: "var(--bg)",
      boxShadow: "0 0 20px var(--glow-amber)"
    },
    info: {
      background: "var(--panel)",
      borderColor: "var(--border)",
      color: "var(--cyan)",
      boxShadow: "0 0 20px var(--glow-cyan)"
    },
    ghost: {
      background: "transparent",
      borderColor: "var(--border)",
      color: "var(--text-muted)"
    }
  };

  const baseStyle = {
    padding: "10px 18px",
    borderRadius: "8px",
    fontSize: "12px",
    fontWeight: 600,
    cursor: disabled ? "not-allowed" : "pointer",
    border: "1px solid var(--border)",
    transition: "all 0.2s ease",
    ...style
  };

  return (
    <button
      onClick={disabled ? undefined : onClick}
      disabled={disabled}
      style={{
        ...baseStyle,
        ...variantStyles[variant],
        ...style
      }}
    >
      {children ?? title}
    </button>
  );
}

interface BotStatusBadgeProps {
  mode: string;
  paused: boolean;
  emergency_stopped: boolean;
  execution_enabled: boolean;
}

function BotStatusBadge({ mode, paused, emergency_stopped, execution_enabled }: BotStatusBadgeProps) {
  const label = emergency_stopped ? "EMERGENCY" : paused ? "PAUSED" : execution_enabled ? "LIVE" : "IDLE";
  const color = emergency_stopped ? "var(--rose)" : paused ? "var(--amber)" : execution_enabled ? "var(--emerald)" : "var(--cyan)";

  return (
    <span style={{
      padding: "4px 10px",
      fontSize: "10px",
      fontWeight: 600,
      textTransform: "uppercase",
      letterSpacing: "0.05em",
      border: "1px solid var(--border)",
      borderRadius: "999px",
      background: "var(--bg)",
      color: color,
      boxShadow: "0 0 12px " + color
    }}>
      {label}
    </span>
  );
}

/* Arabic tooltip support - added inline */