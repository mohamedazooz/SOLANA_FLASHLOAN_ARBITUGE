import { useState } from "react";
import { api } from "../api";

export default function Login({ onLogin }: { onLogin: () => void }) {
  const [password, setPassword] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    setBusy(true);
    setError(null);
    try {
      const { token } = await api.login(password);
      sessionStorage.setItem("arb_session_token", token);
      onLogin();
    } catch (err) {
      setError(err instanceof Error ? err.message : "login failed");
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="login-wrap">
      <form className="login-card" onSubmit={submit}>
        <div className="brand">
          <span className="logo">⚡</span>
          <h1>Solana Flash-Loan Arbitrage</h1>
        </div>
        <p className="muted">Dashboard is read-only until authenticated.</p>
        <input
          type="password"
          placeholder="Operator password"
          value={password}
          onChange={(e) => setPassword(e.target.value)}
          autoFocus
        />
        {error && <div className="error">{error}</div>}
        <button className="btn btn-primary" disabled={busy || !password}>
          {busy ? "Signing in…" : "Sign in"}
        </button>
      </form>
    </div>
  );
}
