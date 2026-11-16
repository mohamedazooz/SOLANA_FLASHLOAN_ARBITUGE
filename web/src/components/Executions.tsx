import { useEffect, useState } from "react";
import { api } from "../api";
import type { ExecutionRecord } from "../api";

export default function Executions() {
  const [rows, setRows] = useState<ExecutionRecord[]>([]);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const load = () =>
      api
        .executions()
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
            <th>Outcome</th>
            <th>Route</th>
            <th>Detail</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((r, i) => (
            <tr key={i}>
              <td className="mono">{new Date(r.at_unix * 1000).toISOString().slice(11, 19)}</td>
              <td>
                <span
                  className={`badge ${
                    r.outcome === "landed"
                      ? "badge-green"
                      : r.outcome === "dropped"
                        ? "badge-red"
                        : "badge-amber"
                  }`}
                >
                  {r.outcome}
                </span>
              </td>
              <td className="mono route">{r.route}</td>
              <td>{r.detail}</td>
            </tr>
          ))}
          {rows.length === 0 && (
            <tr>
              <td colSpan={4} className="muted">
                No executions recorded yet (shadow mode records would appear here).
              </td>
            </tr>
          )}
        </tbody>
      </table>
    </section>
  );
}
