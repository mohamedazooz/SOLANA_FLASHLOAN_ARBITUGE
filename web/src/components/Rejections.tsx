import { useEffect, useState } from "react";
import { api } from "../api";
import type { RejectionRecord } from "../api";

export default function Rejections() {
  const [rows, setRows] = useState<RejectionRecord[]>([]);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const load = () =>
      api
        .rejections()
        .then(setRows)
        .catch((e) => setError(e.message));
    load();
    const t = setInterval(load, 3_000);
    return () => clearInterval(t);
  }, []);

  return (
    <section>
      {error && <div className="error">{error}</div>}
      <table>
        <thead>
          <tr>
            <th>Time (UTC)</th>
            <th>Slot</th>
            <th>Stage</th>
            <th>Route</th>
            <th>Reason</th>
            <th>Gross (SOL)</th>
            <th>Net (SOL)</th>
            <th>Loan (SOL)</th>
            <th>Pools</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((r, i) => (
            <tr key={i}>
              <td className="mono">{new Date(r.at_unix * 1000).toISOString().slice(11, 19)}</td>
              <td className="mono">{r.slot || "—"}</td>
              <td>
                <span className="badge badge-amber">{r.stage}</span>
              </td>
              <td className="mono route">{r.route}</td>
              <td>{r.reason}</td>
              <td className="mono">{r.gross_sol.toFixed(6)}</td>
              <td className="mono">{r.net_sol != null ? r.net_sol.toFixed(6) : "—"}</td>
              <td className="mono">{r.loan_size_sol.toFixed(3)}</td>
              <td className="mono">{r.pools_count}</td>
            </tr>
          ))}
          {rows.length === 0 && (
            <tr>
              <td colSpan={9} className="muted">
                No rejected opportunities recorded yet.
              </td>
            </tr>
          )}
        </tbody>
      </table>
    </section>
  );
}
