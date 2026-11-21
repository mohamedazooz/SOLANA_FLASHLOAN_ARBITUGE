import { useEffect, useRef, useState } from 'react';
import { api, getToken, setToken, type StatusSnapshot } from './api';
import { Sidebar, type TabType } from './components/Sidebar';
import { Login } from './components/Login';
import { Overview } from './components/Overview';
import { Opportunities } from './components/Opportunities';
import { Pools } from './components/Pools';
import { Executions } from './components/Executions';
import { Logs } from './components/Logs';
import Rejections from './components/Rejections';
import StrategyConfig from './components/StrategyConfig';
import Providers from './components/Providers';
import RouteMonitor from './components/RouteMonitor';
import RiskControl from './components/RiskControl';
import EgyptianGuide from './components/EgyptianGuide';
import {
  ShieldAlert,
  ShieldCheck,
  RefreshCw,
} from 'lucide-react';

export default function App() {
  const [authed, setAuthed] = useState<boolean>(!!getToken());
  const [status, setStatus] = useState<StatusSnapshot | null>(null);
  const [currentTab, setCurrentTab] = useState<TabType>('overview');
  const [collapsed, setCollapsed] = useState<boolean>(false);
  const [loading, setLoading] = useState<boolean>(false);
  const timerRef = useRef<number | null>(null);

  const reload = () => {
    setLoading(true);
    api.status()
      .then((data) => {
        setStatus(data);
        setAuthed(true);
      })
      .catch(() => {
        // If unauthorized
      })
      .finally(() => setLoading(false));
  };

  useEffect(() => {
    if (!authed) return;
    reload();
    timerRef.current = window.setInterval(reload, 4000);

    // WebSocket real-time state stream. Browser WebSocket cannot set an
    // Authorization header, so obtain a short-lived single-use ticket.
    let ws: WebSocket | null = null;
    let reconnectTimer: number | null = null;
    let disposed = false;

    const scheduleReconnect = (delayMs: number) => {
      if (disposed) return;
      if (reconnectTimer !== null) window.clearTimeout(reconnectTimer);
      reconnectTimer = window.setTimeout(() => {
        reconnectTimer = null;
        void connect();
      }, delayMs);
    };

    const connect = async () => {
      try {
        const wsUrl = await api.wsUrl();
        if (disposed) return;
        const socket = new WebSocket(wsUrl);
        ws = socket;
        socket.onmessage = (ev) => {
          try {
            const data = JSON.parse(ev.data);
            if (data && typeof data === 'object') {
              if ('mode' in data) {
                setStatus(data as StatusSnapshot);
              } else if (data.kind === 'opportunity' && data.payload?.slot) {
                const s = Number(data.payload.slot);
                setStatus((prev) =>
                  prev
                    ? {
                        ...prev,
                        slot: Math.max(prev.slot, s),
                        profitable_total: prev.profitable_total + (data.payload.eligible ? 1 : 0),
                      }
                    : prev
                );
              }
            }
          } catch {
            // Ignore non-JSON events.
          }
        };
        socket.onclose = () => scheduleReconnect(4000);
        socket.onerror = () => socket.close();
      } catch {
        scheduleReconnect(5000);
      }
    };

    void connect();

    return () => {
      disposed = true;
      if (timerRef.current !== null) window.clearInterval(timerRef.current);
      if (reconnectTimer !== null) window.clearTimeout(reconnectTimer);
      ws?.close();
    };
  }, [authed]);

  const handleLogout = () => {
    api.logout().catch(() => {});
    setToken(null);
    setAuthed(false);
  };

  const handleEmergencyStop = async () => {
    try {
      const result = await api.control('emergency-stop');
      if (!result.ok) throw new Error(result.message || 'Emergency stop was not confirmed');
      reload();
    } catch (e) {
      console.error('Emergency stop request failed:', e);
      reload();
    }
  };

  const handleResumeExecution = async () => {
    try {
      const cleared = await api.control('clear-emergency-stop');
      if (!cleared.ok) throw new Error(cleared.message || 'Emergency latch clear was not confirmed');
      const started = await api.control('start');
      if (!started.ok) throw new Error(started.message || 'Execution start was not confirmed');
      reload();
    } catch (e) {
      console.error('Execution resume request failed:', e);
      reload();
    }
  };

  if (!authed) {
    return <Login onLogin={() => { setAuthed(true); reload(); }} />;
  }

  return (
    <div className="min-h-screen bg-[#06080f] text-slate-100 flex flex-row-reverse" dir="rtl">
      {/* ── Fixed Institutional Sidebar ── */}
      <Sidebar
        currentTab={currentTab}
        onSelectTab={setCurrentTab}
        status={status}
        collapsed={collapsed}
        onToggleCollapse={() => setCollapsed(!collapsed)}
        onLogout={handleLogout}
      />

      {/* ── Main Workspace ── */}
      <main
        className={`flex-1 transition-all duration-300 min-h-screen flex flex-col ${
          collapsed ? 'mr-20' : 'mr-72'
        }`}
      >
        {/* ── Top Command Header ── */}
        <header className="sticky top-0 z-30 flex items-center justify-between px-6 py-3.5 bg-[#080c16]/85 backdrop-blur-xl border-b border-[#162035]">
          <div className="flex items-center gap-4">
            <div className="flex items-center gap-2">
              <span className="w-2.5 h-2.5 rounded-full bg-emerald-400 animate-ping" />
              <span className="text-xs font-mono font-bold text-slate-200">SOLANA MAINNET-BETA</span>
            </div>

            <div className="hidden sm:flex items-center gap-2 px-3 py-1 rounded-lg bg-[#0e1526] border border-[#1a2642] text-xs font-mono">
              <span className="text-slate-400">SLOT:</span>
              <span className="font-bold text-cyan-400">#{status?.slot?.toLocaleString() ?? '---'}</span>
            </div>

            <div className="hidden md:flex items-center gap-2 px-3 py-1 rounded-lg bg-[#0e1526] border border-[#1a2642] text-xs font-mono">
              <span className="text-slate-400">TRACKED:</span>
              <span className="font-bold text-slate-200">{status?.pools_tracked ?? 0} POOLS</span>
            </div>
          </div>

          <div className="flex items-center gap-3">
            <button
              onClick={reload}
              className="p-2 rounded-lg bg-[#0e1526] border border-[#1a2642] text-slate-300 hover:text-cyan-400 hover:bg-[#131d33] transition-colors"
              title="تحديث البيانات"
            >
              <RefreshCw className={`w-4 h-4 ${loading ? 'animate-spin' : ''}`} />
            </button>

            {status?.emergency_stopped ? (
              <button
                onClick={handleResumeExecution}
                className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-bold bg-emerald-500 text-slate-950 hover:bg-emerald-400 shadow-md shadow-emerald-500/20 transition-all"
              >
                <ShieldCheck className="w-4 h-4" />
                استئناف العمل
              </button>
            ) : (
              <button
                onClick={handleEmergencyStop}
                className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-bold bg-rose-500/20 text-rose-400 border border-rose-500/30 hover:bg-rose-500/30 transition-all"
              >
                <ShieldAlert className="w-4 h-4" />
                إيقاف طوارئ
              </button>
            )}
          </div>
        </header>

        {/* ── View Content Area ── */}
        <div className="p-6 flex-1 space-y-6">
          {currentTab === 'overview' && (
            <Overview
              status={status}
              onEmergencyStop={handleEmergencyStop}
              onResumeExecution={handleResumeExecution}
            />
          )}
          {currentTab === 'opportunities' && <Opportunities />}
          {currentTab === 'pools' && <Pools />}
          {currentTab === 'executions' && <Executions />}
          {currentTab === 'logs' && <Logs />}
          {currentTab === 'rejections' && <Rejections />}
          {currentTab === 'risk' && <RiskControl status={status} onChanged={reload} />}
          {currentTab === 'providers' && <Providers />}
          {currentTab === 'routes' && <RouteMonitor />}
          {currentTab === 'strategy' && <StrategyConfig onSaved={reload} />}
          {currentTab === 'guide' && <EgyptianGuide />}
        </div>
      </main>
    </div>
  );
}
