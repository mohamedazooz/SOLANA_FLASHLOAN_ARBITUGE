import { useEffect, useState } from "react";
import { api } from "../api";

interface Overrides {
  min_net_profit_usd: number | null;
  min_borrow_lamports: number | null;
  max_borrow_lamports: number | null;
  max_slippage_bps: number | null;
}

export default function StrategyConfig({ onSaved }: { onSaved: () => void }) {
  const [form, setForm] = useState<Overrides>({
    min_net_profit_usd: null,
    min_borrow_lamports: null,
    max_borrow_lamports: null,
    max_slippage_bps: null,
  });
  const [message, setMessage] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api
      .config()
      .then((cfg) => {
        const ov = (cfg.overrides ?? {}) as Overrides;
        setForm(ov);
      })
      .catch((e) => setError(e.message));
  }, []);

  const save = async () => {
    setError(null);
    setMessage(null);
    try {
      await api.updateConfig({
        min_net_profit_usd: form.min_net_profit_usd,
        min_borrow_lamports: form.min_borrow_lamports,
        max_borrow_lamports: form.max_borrow_lamports,
        max_slippage_bps: form.max_slippage_bps,
      });
      setMessage("Configuration validated and applied (audited).");
      onSaved();
    } catch (e) {
      setError(e instanceof Error ? e.message : "update failed");
    }
  };

  const set = (k: keyof Overrides, v: string) =>
    setForm((f) => ({
      ...f,
      [k]: v === "" ? null : k === "min_net_profit_usd" ? Number(v) : Number(v),
    }));

  return (
    <section>
      <div className="card">
        <h3>Strategy configuration</h3>
        <p className="muted">
          Every change is validated server-side, audited, and applied at the
          supervisor's single tuning boundary. Empty fields keep current values.
        </p>
        <div className="form-grid">
          <label>
            Min net profit (USD)
            <input
              type="number"
              step="0.01"
              min="0"
              value={form.min_net_profit_usd ?? ""}
              onChange={(e) => set("min_net_profit_usd", e.target.value)}
            />
          </label>
          <label>
            Min borrow (lamports)
            <input
              type="number"
              min="0"
              value={form.min_borrow_lamports ?? ""}
              onChange={(e) => set("min_borrow_lamports", e.target.value)}
            />
          </label>
          <label>
            Max borrow (lamports)
            <input
              type="number"
              min="0"
              value={form.max_borrow_lamports ?? ""}
              onChange={(e) => set("max_borrow_lamports", e.target.value)}
            />
          </label>
          <label>
            Max slippage (bps, 1–10000)
            <input
              type="number"
              min="1"
              max="10000"
              value={form.max_slippage_bps ?? ""}
              onChange={(e) => set("max_slippage_bps", e.target.value)}
            />
          </label>
        </div>
        {error && <div className="error">{error}</div>}
        {message && <div className="success">{message}</div>}
        <button className="btn btn-primary" onClick={save}>
          Validate & apply
        </button>
      </div>
    </section>
  );
}
