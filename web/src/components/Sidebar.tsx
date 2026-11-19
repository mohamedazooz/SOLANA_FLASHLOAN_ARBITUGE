import React from 'react';
import {
  Activity,
  Radar,
  Waves,
  Zap,
  Terminal,
  AlertTriangle,
  ShieldAlert,
  Server,
  Network,
  Sliders,
  BookOpen,
  LogOut,
  ChevronLeft,
  ChevronRight,
  Cpu,
  Database,
  Radio,
  Flame,
} from 'lucide-react';
import type { StatusSnapshot } from '../api';

export type TabType =
  | 'overview'
  | 'opportunities'
  | 'pools'
  | 'executions'
  | 'logs'
  | 'rejections'
  | 'risk'
  | 'providers'
  | 'routes'
  | 'strategy'
  | 'guide';

interface SidebarProps {
  currentTab: TabType;
  onSelectTab: (tab: TabType) => void;
  status: StatusSnapshot | null;
  collapsed: boolean;
  onToggleCollapse: () => void;
  onLogout: () => void;
  unseenLogsCount?: number;
}

export const Sidebar: React.FC<SidebarProps> = ({
  currentTab,
  onSelectTab,
  status,
  collapsed,
  onToggleCollapse,
  onLogout,
  unseenLogsCount = 0,
}) => {
  const navItems: { id: TabType; label: string; icon: React.ComponentType<{ className?: string }>; badge?: string | number; badgeColor?: string }[] = [
    { id: 'overview', label: 'نظرة عامة & KPIs', icon: Activity },
    { id: 'opportunities', label: 'رادار الفرص الحية', icon: Radar, badge: status?.profitable_total ? `${status.profitable_total}` : undefined, badgeColor: 'bg-emerald-500/20 text-emerald-400 border border-emerald-500/30' },
    { id: 'pools', label: 'أحواض السيولة', icon: Waves, badge: status?.pools_tracked ? `${status.pools_tracked}` : undefined },
    { id: 'executions', label: 'سجل الصفقات المنفذة', icon: Zap, badge: status?.executed_total ? `${status.executed_total}` : undefined, badgeColor: 'bg-cyan-500/20 text-cyan-400 border border-cyan-500/30' },
    { id: 'logs', label: 'بث اللوجات المباشر', icon: Terminal, badge: unseenLogsCount > 0 ? `${unseenLogsCount}` : undefined, badgeColor: 'bg-amber-500/20 text-amber-400 border border-amber-500/30' },
    { id: 'rejections', label: 'المرفوضات والانزلاق', icon: AlertTriangle, badge: status?.failed_total ? `${status.failed_total}` : undefined, badgeColor: 'bg-rose-500/20 text-rose-400' },
    { id: 'risk', label: 'بوابات الأمان والمخاطر', icon: ShieldAlert },
    { id: 'providers', label: 'مزودات RPC & Jito', icon: Server },
    { id: 'routes', label: 'مصفوفة المسارات HFT', icon: Network },
    { id: 'strategy', label: 'إعدادات الاستراتيجية', icon: Sliders },
    { id: 'guide', label: 'دليل التشغيل السريع', icon: BookOpen },
  ];

  const isLive = status?.mode === 'live' || status?.execution_enabled;
  const isHealthy = status?.stream_connected && status?.helius_connected;

  return (
    <aside
      className={`fixed top-0 right-0 h-screen z-40 flex flex-col justify-between transition-all duration-300 ease-in-out border-l border-[#1a233a] bg-[#090d16] text-slate-100 ${
        collapsed ? 'w-20' : 'w-72'
      }`}
    >
      {/* ── Header ── */}
      <div>
        <div className="flex items-center justify-between p-4 border-b border-[#172138] bg-[#0c1220]">
          {!collapsed && (
            <div className="flex items-center gap-3 overflow-hidden">
              <div className="w-9 h-9 rounded-lg bg-gradient-to-tr from-cyan-500/20 via-[#00f0ff] to-cyan-400 p-[1px] flex items-center justify-center shadow-lg shadow-cyan-500/20">
                <div className="w-full h-full bg-[#090d16] rounded-[7px] flex items-center justify-center">
                  <Flame className="w-5 h-5 text-cyan-400 animate-pulse" />
                </div>
              </div>
              <div>
                <h1 className="text-sm font-extrabold tracking-wider text-slate-100 font-mono uppercase">
                  SENTINEL <span className="text-cyan-400">HFT</span>
                </h1>
                <div className="flex items-center gap-1.5 mt-0.5">
                  <span className={`w-2 h-2 rounded-full ${isHealthy ? 'bg-emerald-400 animate-ping' : 'bg-rose-400'}`} />
                  <span className="text-[10px] font-semibold tracking-wide text-slate-400 uppercase">
                    {isLive ? 'LIVE MAINNET' : 'SHADOW / SIM'}
                  </span>
                </div>
              </div>
            </div>
          )}
          {collapsed && (
            <div className="mx-auto">
              <Flame className="w-6 h-6 text-cyan-400 animate-pulse" />
            </div>
          )}
          <button
            onClick={onToggleCollapse}
            title={collapsed ? 'توسيع القائمة' : 'طي القائمة'}
            className="p-1.5 rounded-lg text-slate-400 hover:text-slate-100 hover:bg-[#151f35] transition-colors"
          >
            {collapsed ? <ChevronLeft className="w-5 h-5" /> : <ChevronRight className="w-5 h-5" />}
          </button>
        </div>

        {/* ── System Status Heartbeat ── */}
        {!collapsed && (
          <div className="p-3 mx-3 mt-3 rounded-lg bg-[#0e1628] border border-[#1b2640] text-xs">
            <div className="text-[10px] uppercase font-mono text-slate-400 font-bold mb-2 flex items-center justify-between">
              <span>حالة الشبكة والخوادم</span>
              <span className="text-cyan-400 font-mono">SLOT #{status?.slot ?? '---'}</span>
            </div>
            <div className="grid grid-cols-2 gap-2">
              <div className="flex items-center gap-2 p-1.5 rounded bg-[#090e1a] border border-slate-800">
                <Cpu className="w-3.5 h-3.5 text-cyan-400" />
                <span className="text-[11px] text-slate-300">Arb Bot</span>
                <span className="w-1.5 h-1.5 rounded-full bg-emerald-400 ml-auto" />
              </div>
              <div className="flex items-center gap-2 p-1.5 rounded bg-[#090e1a] border border-slate-800">
                <Radio className="w-3.5 h-3.5 text-amber-400" />
                <span className="text-[11px] text-slate-300">Helius WS</span>
                <span className={`w-1.5 h-1.5 rounded-full ml-auto ${status?.helius_connected ? 'bg-emerald-400' : 'bg-rose-400'}`} />
              </div>
              <div className="flex items-center gap-2 p-1.5 rounded bg-[#090e1a] border border-slate-800">
                <Database className="w-3.5 h-3.5 text-purple-400" />
                <span className="text-[11px] text-slate-300">Redis 6379</span>
                <span className="w-1.5 h-1.5 rounded-full bg-emerald-400 ml-auto" />
              </div>
              <div className="flex items-center gap-2 p-1.5 rounded bg-[#090e1a] border border-slate-800">
                <Server className="w-3.5 h-3.5 text-emerald-400" />
                <span className="text-[11px] text-slate-300">Jito Leader</span>
                <span className="w-1.5 h-1.5 rounded-full bg-emerald-400 ml-auto" />
              </div>
            </div>
          </div>
        )}

        {/* ── Navigation Links ── */}
        <nav className="p-3 space-y-1.5 overflow-y-auto max-h-[calc(100vh-270px)]">
          {navItems.map((item) => {
            const Icon = item.icon;
            const isActive = currentTab === item.id;
            return (
              <button
                key={item.id}
                onClick={() => onSelectTab(item.id)}
                title={collapsed ? item.label : undefined}
                className={`w-full flex items-center gap-3 px-3 py-2.5 rounded-lg text-sm font-medium transition-all ${
                  isActive
                    ? 'bg-gradient-to-r from-cyan-500/20 to-transparent text-cyan-300 border-r-4 border-cyan-400 font-semibold'
                    : 'text-slate-400 hover:text-slate-200 hover:bg-[#121a2d]'
                }`}
              >
                <Icon className={`w-5 h-5 flex-shrink-0 ${isActive ? 'text-cyan-400' : 'text-slate-400'}`} />
                {!collapsed && <span className="truncate">{item.label}</span>}
                {!collapsed && item.badge !== undefined && (
                  <span
                    className={`ml-auto px-2 py-0.5 text-[10px] font-mono rounded-full font-bold ${
                      item.badgeColor || 'bg-slate-800 text-slate-300'
                    }`}
                  >
                    {item.badge}
                  </span>
                )}
              </button>
            );
          })}
        </nav>
      </div>

      {/* ── Footer / Logout ── */}
      <div className="p-3 border-t border-[#172138] bg-[#0c1220]">
        {!collapsed && (
          <div className="mb-3 px-2 py-1.5 rounded bg-[#070b14] border border-[#1b253b] flex items-center justify-between text-xs font-mono">
            <span className="text-slate-400">صافي PnL:</span>
            <span className={`font-bold ${((status?.net_pnl_sol ?? 0) >= 0) ? 'text-emerald-400' : 'text-rose-400'}`}>
              {(status?.net_pnl_sol ?? 0).toFixed(4)} SOL
            </span>
          </div>
        )}
        <button
          onClick={onLogout}
          title="تسجيل الخروج"
          className="w-full flex items-center justify-center gap-2 px-3 py-2 rounded-lg text-xs font-semibold text-rose-400 hover:bg-rose-500/10 border border-rose-500/20 transition-colors"
        >
          <LogOut className="w-4 h-4 flex-shrink-0" />
          {!collapsed && <span>تسجيل الخروج</span>}
        </button>
      </div>
    </aside>
  );
};
