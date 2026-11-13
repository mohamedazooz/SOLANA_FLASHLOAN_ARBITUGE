// API client for the arb-bot web backend. The session token lives in
// sessionStorage (cleared on tab close); it is sent as a Bearer header and as
// `?token=` for the WebSocket handshake.

const TOKEN_KEY = "arb_session_token";

export function getToken(): string | null {
  return sessionStorage.getItem(TOKEN_KEY);
}

export function setToken(token: string | null) {
  if (token) sessionStorage.setItem(TOKEN_KEY, token);
  else sessionStorage.removeItem(TOKEN_KEY);
}

export class ApiError extends Error {
  status: number;
  constructor(status: number, message: string) {
    super(message);
    this.status = status;
  }
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const token = getToken();
  const headers: Record<string, string> = {
    "Content-Type": "application/json",
    ...(init?.headers as Record<string, string> | undefined),
  };
  if (token) headers["Authorization"] = `Bearer ${token}`;
  const resp = await fetch(path, { ...init, headers });
  if (resp.status === 401) {
    setToken(null);
    throw new ApiError(401, "unauthorized");
  }
  if (!resp.ok) {
    const body = await resp.json().catch(() => ({}));
    throw new ApiError(resp.status, body.error ?? `HTTP ${resp.status}`);
  }
  return resp.json() as Promise<T>;
}

export interface StatusSnapshot {
  mode: string;
  execution_enabled: boolean;
  paused: boolean;
  emergency_stopped: boolean;
  slot: number;
  pools_tracked: number;
  stream_connected: boolean;
  helius_connected: boolean;
  opportunities_total: number;
  profitable_total: number;
  executed_total: number;
  successful_total: number;
  failed_total: number;
  net_pnl_sol: number;
  uptime_secs: number;
}

export interface OpportunityRecord {
  captured_at_unix: number;
  slot: number;
  route: string;
  protocol: string;
  borrow_lamports: number;
  expected_output_lamports: number;
  net_profit_lamports: number;
  eligible: boolean;
  reject_reason: string | null;
}

export interface ExecutionRecord {
  at_unix: number;
  outcome: string;
  route: string;
  detail: string;
}

export interface LogRecord {
  at_unix: number;
  level: string;
  stage: string;
  message: string;
}

export interface ProtocolCoverage {
  protocol: string;
  pools: number;
  avg_fee_bps: number;
}

export const api = {
  login: (password: string) =>
    request<{ token: string; expires_at_unix: number }>("/api/login", {
      method: "POST",
      body: JSON.stringify({ password }),
    }),
  logout: () => request<{ ok: boolean }>("/api/logout", { method: "POST" }),
  status: () => request<StatusSnapshot>("/api/status"),
  opportunities: () => request<OpportunityRecord[]>("/api/opportunities"),
  executions: () => request<ExecutionRecord[]>("/api/executions"),
  logs: () => request<LogRecord[]>("/api/logs"),
  pnl: () => request<Record<string, unknown>>("/api/pnl"),
  providers: () => request<Record<string, unknown>>("/api/providers"),
  routes: () => request<ProtocolCoverage[]>("/api/routes"),
  config: () => request<Record<string, unknown>>("/api/config"),
  updateConfig: (overrides: Record<string, unknown>) =>
    request<{ ok: boolean }>("/api/config", {
      method: "POST",
      body: JSON.stringify(overrides),
    }),
  control: (action: string, confirm?: string) =>
    request<{ ok: boolean; message: string }>(`/api/control/${action}`, {
      method: "POST",
      body: JSON.stringify(confirm ? { confirm } : {}),
    }),
  wsTicket: () =>
    request<{ ticket: string; expires_at_unix: number }>("/api/ws-ticket", {
      method: "POST",
    }),
  wsUrl: async () => {
    // Single-use ticket: the long-lived session token never appears in a URL.
    const { ticket } = await api.wsTicket();
    const proto = location.protocol === "https:" ? "wss" : "ws";
    return `${proto}://${location.host}/api/ws?ticket=${encodeURIComponent(ticket)}`;
  },
};

export function lamportsToSol(v: number): string {
  return (v / 1e9).toFixed(6);
}
