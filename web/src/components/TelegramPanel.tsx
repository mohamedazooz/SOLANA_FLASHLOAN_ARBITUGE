import { useEffect, useState } from "react";
import { api } from "../api";

export default function TelegramPanel() {
  const [config, setConfig] = useState<Record<string, unknown> | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api
      .config()
      .then(setConfig)
      .catch((e) => setError(e.message));
  }, []);

  const controlConfigured = !!config;

  return (
    <section>
      {error && <div className="error">{error}</div>}
      <div className="card">
        <h3>Telegram control</h3>
        <p className="muted">
          Telegram and this web application share the SAME control service: the
          commands <code>/status /start /stop /shadow /paper /live /opportunities
          /pnl /providers /config</code> execute identical authorization,
          validation and audit paths on both interfaces.
        </p>
        <div className="kv">
          <span>Control listener</span>
          <span className={controlConfigured ? "tone-green" : "tone-red"}>
            {controlConfigured ? "configured (token + chat allowlist)" : "not configured"}
          </span>
        </div>
        <div className="kv">
          <span>Authorization</span>
          <span className="mono">TELEGRAM_CONTROL_CHAT_IDS allowlist</span>
        </div>
      </div>
      <div className="card">
        <h3>Recent commands</h3>
        <p className="muted">
          Command history is written to the shared audit log
          (<code>WEB_AUDIT_LOG</code>, default <code>data/web_audit.log</code>) with the
          same entries as web actions.
        </p>
      </div>
    </section>
  );
}
