import type { StatusSnapshot } from "../api";

function Stat({ label, value, tone }: { label: string; value: string; tone?: string }) {
  return (
    <div className={`card stat ${tone ?? ""}`}>
      <div className="stat-value">{value}</div>
      <div className="stat-label">{label}</div>
    </div>
  );
}

export default function Overview({ status }: { status: StatusSnapshot | null }) {
  if (!status) return <div className="muted">Loading status…</div>;
  return (
    <section>
      <div className="stat-grid">
        <Stat label="Mode" value={status.mode} />
        <Stat
          label="Execution"
          value={status.paused ? "PAUSED" : status.execution_enabled ? "enabled" : "disabled"}
          tone={status.paused ? "tone-red" : "tone-green"}
        />
        <Stat label="Network slot" value={status.slot.toLocaleString()} />
        <Stat label="Pools tracked" value={String(status.pools_tracked)} />
        <Stat
          label="Primary stream"
          value={status.stream_connected ? "connected" : "disconnected"}
          tone={status.stream_connected ? "tone-green" : "tone-red"}
        />
        <Stat
          label="Helius"
          value={status.helius_connected ? "connected" : "disconnected"}
          tone={status.helius_connected ? "tone-green" : "tone-red"}
        />
        <Stat label="Opportunities" value={status.opportunities_total.toLocaleString()} />
        <Stat label="Profitable" value={status.profitable_total.toLocaleString()} />
        <Stat label="Executed" value={status.executed_total.toLocaleString()} />
        <Stat label="Successful" value={status.successful_total.toLocaleString()} tone="tone-green" />
        <Stat label="Failed" value={status.failed_total.toLocaleString()} tone="tone-red" />
        <Stat
          label="Net PnL (SOL)"
          value={status.net_pnl_sol.toFixed(6)}
          tone={status.net_pnl_sol >= 0 ? "tone-green" : "tone-red"}
        />
      </div>
      <div className="card">
        <h3>Session</h3>
        <div className="kv">
          <span>Uptime</span>
          <span className="mono">{Math.floor(status.uptime_secs / 60)}m {status.uptime_secs % 60}s</span>
        </div>
        <div className="kv">
          <span>Emergency stop</span>
          <span className={status.emergency_stopped ? "tone-red" : "tone-green"}>
            {status.emergency_stopped ? "ENGAGED" : "clear"}
          </span>
        </div>
      </div>
    </section>
  );
}
