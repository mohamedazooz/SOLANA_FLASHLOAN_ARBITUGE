import { useEffect, useState } from "react";
import { api } from "../api";

interface RpcProviderHealth {
  label?: string;
  status?: string;
  ok_count?: number;
  rate_limited_429?: number;
  error_count?: number;
  quota_exhausted?: boolean;
  cooldown_remaining_secs?: number;
  p50_latency_ms?: number;
  p95_latency_ms?: number;
  last_error?: string | null;
}

interface RpcPoolHealth {
  total?: number;
  healthy_count?: number;
  degraded_count?: number;
  quota_exhausted_count?: number;
  in_cooldown?: number;
  total_ok?: number;
  total_429?: number;
  total_errors?: number;
  providers?: RpcProviderHealth[];
}

export default function Providers() {
  const [data, setData] = useState<Record<string, unknown> | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const load = () =>
      api
        .providers()
        .then(setData)
        .catch((e) => setError(e.message));
    load();
    const t = setInterval(load, 3_000);
    return () => clearInterval(t);
  }, []);

  const entries = Object.entries(data ?? {}).filter(
    ([name]) => name !== "rpc_pool" && name !== "subscriptions_active",
  );
  const pool = (data?.rpc_pool ?? null) as RpcPoolHealth | null;
  const subsActive = Number(data?.subscriptions_active ?? 0);

  return (
    <section>
      {error && <div className="error">{error}</div>}
      <div className="stat-grid">
        {entries.map(([name, info]) => {
          const connected = (info as { connected?: boolean })?.connected;
          const slot = (info as { slot?: number })?.slot;
          return (
            <div key={name} className={`card stat ${connected ? "tone-green" : "tone-red"}`}>
              <div className="stat-value">{connected ? "✓ connected" : "✗ down"}</div>
              <div className="stat-label">
                {name} {slot ? `· slot ${slot.toLocaleString()}` : ""}
              </div>
            </div>
          );
        })}
        <div className={`card stat ${subsActive > 0 ? "tone-green" : "tone-red"}`}>
          <div className="stat-value">{subsActive}</div>
          <div className="stat-label">Active WS subscriptions</div>
        </div>
      </div>

      {pool && pool.providers && pool.providers.length > 0 && (
        <div className="card">
          <h3>RPC pool — quota utilization</h3>
          <table>
            <thead>
              <tr>
                <th>Provider</th>
                <th>Status</th>
                <th>OK</th>
                <th>429s</th>
                <th>Errors</th>
                <th>Quota</th>
                <th>Cooldown</th>
                <th>p50/p95 ms</th>
              </tr>
            </thead>
            <tbody>
              {pool.providers.map((p, i) => (
                <tr key={i}>
                  <td>{p.label ?? `#${i}`}</td>
                  <td>{p.status ?? "unknown"}</td>
                  <td className="mono">{p.ok_count ?? 0}</td>
                  <td className={`mono ${(p.rate_limited_429 ?? 0) > 0 ? "tone-red" : ""}`}>
                    {p.rate_limited_429 ?? 0}
                  </td>
                  <td className="mono">{p.error_count ?? 0}</td>
                  <td>
                    {p.quota_exhausted ? (
                      <span className="badge badge-red">exhausted</span>
                    ) : (
                      <span className="badge badge-green">ok</span>
                    )}
                  </td>
                  <td className="mono">{p.cooldown_remaining_secs ? `${p.cooldown_remaining_secs}s` : "—"}</td>
                  <td className="mono">
                    {p.p50_latency_ms ?? "—"}/{p.p95_latency_ms ?? "—"}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}

      <div className="card">
        <h3>Provider policy</h3>
        <ul className="muted">
          <li>Primary market data: WebSocket stream (Solami / Helius) — no stale-cache fallback.</li>
          <li>Helius LaserStream (Yellowstone gRPC) takes precedence when configured.</li>
          <li>HTTP RPC is reserved for operations: account lookups, simulation, submission.</li>
          <li>Bounded retries with provider rotation; fail-closed when all providers fail.</li>
        </ul>
      </div>
    </section>
  );
}
