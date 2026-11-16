import { useState } from "react";
import { api } from "../api";
import type { StatusSnapshot } from "../api";

export default function RiskControl({
  status,
  onChanged,
}: {
  status: StatusSnapshot | null;
  onChanged: () => void;
}) {
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [confirmText, setConfirmText] = useState("");

  const run = async (fn: () => Promise<string>) => {
    setBusy(true);
    setError(null);
    setMessage(null);
    try {
      setMessage(await fn());
      onChanged();
    } catch (e) {
      setError(e instanceof Error ? e.message : "action failed");
    } finally {
      setBusy(false);
    }
  };

  return (
    <section>
      <div className="card">
        <h3>Execution control</h3>
        <p className="muted">
          Current mode: <b>{status?.mode ?? "…"}</b> · execution{" "}
          <b>{status?.paused ? "PAUSED" : status?.execution_enabled ? "enabled" : "disabled"}</b>
        </p>
        <div className="btn-row">
          <button className="btn btn-green" disabled={busy} onClick={() => run(() => api.control("start").then((r) => r.message))}>
            ▶ Start execution
          </button>
          <button className="btn btn-amber" disabled={busy} onClick={() => run(() => api.control("stop").then((r) => r.message))}>
            ⏸ Stop execution
          </button>
          <button className="btn" disabled={busy} onClick={() => run(() => api.control("shadow").then((r) => r.message))}>
            Shadow mode
          </button>
          <button className="btn" disabled={busy} onClick={() => run(() => api.control("paper").then((r) => r.message))}>
            Paper mode
          </button>
        </div>
      </div>

      <div className="card danger-zone">
        <h3>⚠ Live mode (explicit confirmation required)</h3>
        <p className="muted">
          Requires <code>ARB_LIVE=1</code> in the bot environment. Type
          <code> CONFIRM-LIVE </code> below to authorize real funds execution.
        </p>
        <input
          placeholder="CONFIRM-LIVE"
          value={confirmText}
          onChange={(e) => setConfirmText(e.target.value)}
        />
        <button
          className="btn btn-red"
          disabled={busy || confirmText !== "CONFIRM-LIVE"}
          onClick={() => run(() => api.control("live", confirmText).then((r) => r.message))}
        >
          Switch to live mode
        </button>
      </div>

      <div className="card danger-zone">
        <h3>🛑 Emergency stop</h3>
        <p className="muted">
          Latches execution off until explicitly cleared — even across start
          commands from any interface (web or Telegram).
        </p>
        <div className="btn-row">
          <button
            className="btn btn-red"
            disabled={busy}
            onClick={() => run(() => api.control("emergency-stop").then((r) => r.message))}
          >
            ENGAGE emergency stop
          </button>
          <button
            className="btn btn-amber"
            disabled={busy}
            onClick={() => run(() => api.control("clear-emergency-stop").then((r) => r.message))}
          >
            Clear latch
          </button>
        </div>
      </div>

      {error && <div className="error">{error}</div>}
      {message && <div className="success">{message}</div>}
    </section>
  );
}
