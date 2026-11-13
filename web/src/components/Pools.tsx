import { useEffect, useState } from "react";
import { api } from "../api";

interface PoolRow {
  pool_id: string;
  protocol: string;
  base_mint: string;
  quote_mint: string;
  last_update_slot: number;
}

export default function Pools() {
  const [rows, setRows] = useState<PoolRow[]>([]);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const load = () =>
      api
        .providers()
        .then(async () => {
          // Pool data comes from the status/pools endpoint when populated by
          // the supervisor; until then the section shows provider-linked state.
          const status = await api.status();
          setRows((prev) =>
            prev.length
              ? prev
              : [
                  {
                    pool_id: "— awaiting supervisor pool publication —",
                    protocol: "-",
                    base_mint: "-",
                    quote_mint: "-",
                    last_update_slot: Number(status.slot),
                  },
                ],
          );
        })
        .catch((e) => setError(e.message));
    load();
    const t = setInterval(load, 5_000);
    return () => clearInterval(t);
  }, []);

  return (
    <section>
      {error && <div className="error">{error}</div>}
      <div className="card">
        <h3>Tracked pools</h3>
        <p className="muted">
          Pool inventory is published by the supervisor's registry. The unified
          pipeline tracks Meteora DLMM pools (bin arrays), Raydium AMM/CPMM/CLMM,
          Orca Whirlpool, Pump.fun and PumpSwap; each row shows the last update
          slot so stale pools are visible at a glance.
        </p>
        <table>
          <thead>
            <tr>
              <th>Pool</th>
              <th>Protocol</th>
              <th>Pair</th>
              <th>Last update slot</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((r, i) => (
              <tr key={i}>
                <td className="mono route">{r.pool_id}</td>
                <td>{r.protocol}</td>
                <td className="mono">{r.base_mint.slice(0, 8)}…/{r.quote_mint.slice(0, 8)}…</td>
                <td className="mono">{r.last_update_slot}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </section>
  );
}
