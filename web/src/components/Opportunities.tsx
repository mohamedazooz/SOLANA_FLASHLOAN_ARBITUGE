import { useEffect, useState } from "react";
import { api, lamportsToSol } from "../api";
import type { OpportunityRecord } from "../api";

function shortPool(p: string) {
  return p.length > 8 ? `${p.slice(0, 4)}…${p.slice(-4)}` : p;
}

export default function Opportunities() {
  const [rows, setRows] = useState<OpportunityRecord[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [protocol, setProtocol] = useState("");
  const [onlyEligible, setOnlyEligible] = useState(false);
  const [minNet, setMinNet] = useState("");

  useEffect(() => {
    const load = () =>
      api
        .opportunities()
        .then(setRows)
        .catch((e) => setError(e.message));
    load();
    const t = setInterval(load, 3_000);
    return () => clearInterval(t);
  }, []);

  const filtered = rows.filter((r) => {
    if (protocol && !r.protocol.includes(protocol)) return false;
    if (onlyEligible && !r.eligible) return false;
    if (minNet && r.net_profit_lamports < Number(minNet) * 1e9) return false;
    return true;
  });

  return (
    <section>
      <div className="filters">
        <input placeholder="Protocol filter" value={protocol} onChange={(e) => setProtocol(e.target.value)} />
        <label className="check">
          <input type="checkbox" checked={onlyEligible} onChange={(e) => setOnlyEligible(e.target.checked)} />
          Eligible only
        </label>
        <input
          placeholder="Min net profit (SOL)"
          value={minNet}
          onChange={(e) => setMinNet(e.target.value)}
        />
      </div>
      {error && <div className="error">{error}</div>}
      <table>
        <thead>
          <tr>
            <th>Time (UTC)</th>
            <th>Slot</th>
            <th>Route</th>
            <th>Protocol</th>
            <th>Borrow (SOL)</th>
            <th>Expected out</th>
            <th>Gross profit</th>
            <th>Net profit (SOL)</th>
            <th>Per-hop quotes</th>
            <th>Status</th>
          </tr>
        </thead>
        <tbody>
          {filtered.map((r, i) => (
            <tr key={i}>
              <td className="mono">{new Date(r.captured_at_unix * 1000).toISOString().slice(11, 19)}</td>
              <td className="mono">{r.slot}</td>
              <td className="mono route">{r.route}</td>
              <td>{r.protocol}</td>
              <td className="mono">{lamportsToSol(r.borrow_lamports)}</td>
              <td className="mono">{lamportsToSol(r.expected_output_lamports)}</td>
              <td className="mono">
                {r.gross_profit_lamports !== undefined
                  ? lamportsToSol(r.gross_profit_lamports)
                  : lamportsToSol(r.expected_output_lamports - r.borrow_lamports)}
              </td>
              <td className={`mono ${r.net_profit_lamports >= 0 ? "tone-green" : "tone-red"}`}>
                {lamportsToSol(r.net_profit_lamports)}
              </td>
              <td>
                {r.hop_quotes && r.hop_quotes.length > 0 ? (
                  <div>
                    {r.hop_quotes.map((h, j) => (
                      <div
                        key={j}
                        className="mono"
                        title={`${h.protocol} pool ${h.pool}`}
                      >
                        {shortPool(h.pool)}: {lamportsToSol(h.amount_in)}→{lamportsToSol(h.amount_out)}
                      </div>
                    ))}
                  </div>
                ) : (
                  "—"
                )}
              </td>
              <td>
                {r.eligible ? (
                  <span className="badge badge-green">eligible</span>
                ) : (
                  <span className="badge badge-red">{r.reject_reason ?? "rejected"}</span>
                )}
              </td>
            </tr>
          ))}
          {filtered.length === 0 && (
            <tr>
              <td colSpan={10} className="muted">
                No opportunities recorded yet.
              </td>
            </tr>
          )}
        </tbody>
      </table>
    </section>
  );
}
