import { useEffect, useState } from "react";
import { api } from "../api";
import type { ProtocolCoverage } from "../api";

export default function RouteMonitor() {
  const [rows, setRows] = useState<ProtocolCoverage[]>([]);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const load = () =>
      api
        .routes()
        .then(setRows)
        .catch((e) => setError(e.message));
    load();
    const t = setInterval(load, 5_000);
    return () => clearInterval(t);
  }, []);

  const total = rows.reduce((acc, r) => acc + r.pools, 0);

  return (
    <section>
      {error && <div className="error">{error}</div>}
      <div className="card">
        <h3>Route coverage by protocol</h3>
        <p className="muted">
          Published by the supervisor every cycle from the live pool registry.
          Every protocol below is routable through the unified multi-hop graph:
          cycles are enumerated across protocols and priced with each pool's own
          fee model. A protocol with zero pools contributes no edges.
        </p>
        {rows.length === 0 ? (
          <p className="muted">No pools registered yet — awaiting supervisor publication.</p>
        ) : (
          <table>
            <thead>
              <tr>
                <th>Protocol</th>
                <th>Pools</th>
                <th>Share</th>
                <th>Avg fee (bps)</th>
                <th>Connectivity</th>
              </tr>
            </thead>
            <tbody>
              {rows.map((r) => (
                <tr key={r.protocol}>
                  <td>{r.protocol}</td>
                  <td>{r.pools}</td>
                  <td>{total > 0 ? `${((r.pools / total) * 100).toFixed(1)}%` : "—"}</td>
                  <td>{r.avg_fee_bps.toFixed(1)}</td>
                  <td className={r.pools > 0 ? "tone-green" : "tone-red"}>
                    {r.pools > 0 ? "routable" : "no edges"}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
        <p className="muted">
          Total tracked pools: {total}. Cross-protocol cycles are discovered by
          the unified graph path (up to the configured max route length) and
          quoted per hop by the protocol adapter registry, with each pool's own
          fee carried into the net-profit calculation.
        </p>
      </div>
    </section>
  );
}
