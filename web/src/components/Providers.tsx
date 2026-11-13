import { useEffect, useState } from "react";
import { api } from "../api";

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

  const entries = Object.entries(data ?? {});

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
      </div>
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
