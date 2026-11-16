import { useEffect, useState } from "react";
import { api } from "../api";
import type { LogRecord } from "../api";

export default function Logs() {
  const [rows, setRows] = useState<LogRecord[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [level, setLevel] = useState("");

  useEffect(() => {
    const load = () =>
      api
        .logs()
        .then(setRows)
        .catch((e) => setError(e.message));
    load();
    const t = setInterval(load, 3_000);
    return () => clearInterval(t);
  }, []);

  const filtered = rows.filter((r) => !level || r.level === level);
  const successRate = (() => {
    const total = rows.length;
    if (!total) return null;
    return null;
  })();
  void successRate;

  return (
    <section>
      <div className="filters">
        <select value={level} onChange={(e) => setLevel(e.target.value)}>
          <option value="">All levels</option>
          <option value="info">info</option>
          <option value="warn">warn</option>
          <option value="error">error</option>
        </select>
      </div>
      {error && <div className="error">{error}</div>}
      <table>
        <thead>
          <tr>
            <th>Time (UTC)</th>
            <th>Level</th>
            <th>Stage</th>
            <th>Message</th>
          </tr>
        </thead>
        <tbody>
          {filtered.map((r, i) => (
            <tr key={i}>
              <td className="mono">{new Date(r.at_unix * 1000).toISOString().slice(11, 19)}</td>
              <td>
                <span
                  className={`badge ${
                    r.level === "error"
                      ? "badge-red"
                      : r.level === "warn"
                        ? "badge-amber"
                        : "badge-green"
                  }`}
                >
                  {r.level}
                </span>
              </td>
              <td>{r.stage}</td>
              <td className="route">{r.message}</td>
            </tr>
          ))}
          {filtered.length === 0 && (
            <tr>
              <td colSpan={4} className="muted">
                No log events yet.
              </td>
            </tr>
          )}
        </tbody>
      </table>
    </section>
  );
}
