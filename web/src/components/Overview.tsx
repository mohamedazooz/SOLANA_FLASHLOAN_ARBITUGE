import React from 'react';
import {
  TrendingUp,
  Zap,
  Target,
  DollarSign,
  Radio,
  Clock,
  ShieldCheck,
  ShieldAlert,
  Layers,
  ArrowUpRight,
  Activity,
  AlertOctagon,
} from 'lucide-react';
import type { StatusSnapshot } from '../api';

interface OverviewProps {
  status: StatusSnapshot | null;
  onEmergencyStop?: () => void;
  onResumeExecution?: () => void;
}

export const Overview: React.FC<OverviewProps> = ({
  status,
  onEmergencyStop,
  onResumeExecution,
}) => {
  if (!status) {
    return (
      <div className="flex flex-col items-center justify-center p-16 text-slate-500">
        <Activity className="w-10 h-10 text-cyan-400 animate-spin mb-4" />
        <p className="text-sm font-mono tracking-wide">جاري قراءة حالة المحرك المباشرة (Connecting to HFT Engine)...</p>
      </div>
    );
  }

  const successRate =
    status.executed_total > 0
      ? ((status.successful_total / status.executed_total) * 100).toFixed(1)
      : '100.0';

  const isProfitable = (status.net_pnl_sol ?? 0) >= 0;

  return (
    <div className="space-y-6">
      {/* ── Top Hero Stats Bar ── */}
      <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4 gap-4">
        {/* Card 1: Net PnL */}
        <div className="p-5 rounded-2xl bg-gradient-to-br from-[#0c1324] to-[#080d19] border border-[#1a2846] relative overflow-hidden shadow-xl">
          <div className="flex items-center justify-between mb-3">
            <span className="text-xs font-bold uppercase tracking-wider text-slate-400">صافي الأرباح (Net PnL)</span>
            <div className={`p-2 rounded-xl ${isProfitable ? 'bg-emerald-500/10 text-emerald-400' : 'bg-rose-500/10 text-rose-400'}`}>
              <DollarSign className="w-5 h-5" />
            </div>
          </div>
          <div className="flex items-baseline gap-2">
            <span className={`text-2xl font-extrabold font-mono tracking-tight ${isProfitable ? 'text-emerald-400' : 'text-rose-400'}`}>
              {status.net_pnl_sol > 0 ? `+${status.net_pnl_sol.toFixed(5)}` : status.net_pnl_sol.toFixed(5)}
            </span>
            <span className="text-xs font-bold text-slate-400 font-mono">SOL</span>
          </div>
          <div className="flex items-center gap-1.5 mt-2 text-xs text-slate-400">
            <TrendingUp className="w-3.5 h-3.5 text-emerald-400" />
            <span>تنفيذ ذري خالٍ من الخسارة (Atomic Revert)</span>
          </div>
          <div className="absolute -bottom-6 -left-6 w-24 h-24 bg-emerald-500/5 rounded-full blur-xl pointer-events-none" />
        </div>

        {/* Card 2: Opportunities & Execution */}
        <div className="p-5 rounded-2xl bg-gradient-to-br from-[#0c1324] to-[#080d19] border border-[#1a2846] relative overflow-hidden shadow-xl">
          <div className="flex items-center justify-between mb-3">
            <span className="text-xs font-bold uppercase tracking-wider text-slate-400">رادار الفرص المكتشفة</span>
            <div className="p-2 rounded-xl bg-cyan-500/10 text-cyan-400">
              <Target className="w-5 h-5" />
            </div>
          </div>
          <div className="flex items-baseline gap-2">
            <span className="text-2xl font-extrabold font-mono tracking-tight text-cyan-400">
              {status.opportunities_total.toLocaleString()}
            </span>
            <span className="text-xs text-emerald-400 font-semibold font-mono">
              ({status.profitable_total} رابحة)
            </span>
          </div>
          <div className="flex items-center gap-1.5 mt-2 text-xs text-slate-400">
            <ArrowUpRight className="w-3.5 h-3.5 text-cyan-400" />
            <span>فحص أسعار الحسابات اللحظي</span>
          </div>
        </div>

        {/* Card 3: Execution Success Rate */}
        <div className="p-5 rounded-2xl bg-gradient-to-br from-[#0c1324] to-[#080d19] border border-[#1a2846] relative overflow-hidden shadow-xl">
          <div className="flex items-center justify-between mb-3">
            <span className="text-xs font-bold uppercase tracking-wider text-slate-400">معدل نجاح الصفقات</span>
            <div className="p-2 rounded-xl bg-purple-500/10 text-purple-400">
              <Zap className="w-5 h-5" />
            </div>
          </div>
          <div className="flex items-baseline gap-2">
            <span className="text-2xl font-extrabold font-mono tracking-tight text-slate-100">
              {successRate}%
            </span>
            <span className="text-xs text-slate-400 font-mono">
              ({status.successful_total}/{status.executed_total})
            </span>
          </div>
          <div className="w-full bg-[#141e33] h-1.5 rounded-full mt-3 overflow-hidden">
            <div
              className="bg-gradient-to-r from-purple-500 to-cyan-400 h-full rounded-full transition-all duration-500"
              style={{ width: `${Math.min(100, Math.max(5, Number(successRate)))}%` }}
            />
          </div>
        </div>

        {/* Card 4: Pools & Network Slot */}
        <div className="p-5 rounded-2xl bg-gradient-to-br from-[#0c1324] to-[#080d19] border border-[#1a2846] relative overflow-hidden shadow-xl">
          <div className="flex items-center justify-between mb-3">
            <span className="text-xs font-bold uppercase tracking-wider text-slate-400">الأحواض المتتبعة & Slot</span>
            <div className="p-2 rounded-xl bg-amber-500/10 text-amber-400">
              <Layers className="w-5 h-5" />
            </div>
          </div>
          <div className="flex items-baseline gap-2">
            <span className="text-2xl font-extrabold font-mono tracking-tight text-slate-100">
              {status.pools_tracked}
            </span>
            <span className="text-xs font-mono text-cyan-400">
              #{status.slot.toLocaleString()}
            </span>
          </div>
          <div className="flex items-center gap-1.5 mt-2 text-xs text-slate-400">
            <Radio className="w-3.5 h-3.5 text-amber-400 animate-pulse" />
            <span>Raydium + CLMM + Orca + DLMM</span>
          </div>
        </div>
      </div>

      {/* ── Operational Control & Security Gates ── */}
      <div className="p-5 rounded-2xl bg-[#090d18] border border-[#18233d] shadow-xl">
        <div className="flex flex-wrap items-center justify-between gap-4 pb-4 border-b border-[#152038]">
          <div className="flex items-center gap-3">
            <div className={`p-2.5 rounded-xl ${status.emergency_stopped ? 'bg-rose-500/20 text-rose-400' : 'bg-emerald-500/20 text-emerald-400'}`}>
              {status.emergency_stopped ? <ShieldAlert className="w-6 h-6" /> : <ShieldCheck className="w-6 h-6" />}
            </div>
            <div>
              <h3 className="text-base font-bold text-slate-100 flex items-center gap-2">
                وضع التحكم والتشغيل (Engine Control Mode)
                <span className={`px-2.5 py-0.5 rounded-full text-[11px] font-mono font-bold uppercase ${
                  status.mode === 'live' ? 'bg-emerald-500/20 text-emerald-400 border border-emerald-500/30' : 'bg-amber-500/20 text-amber-400 border border-amber-500/30'
                }`}>
                  {status.mode}
                </span>
              </h3>
              <p className="text-xs text-slate-400 mt-0.5">
                حالة التنفيذ: {status.paused ? 'متوقف مؤقتاً (PAUSED)' : status.execution_enabled ? 'مفعل وجاهز لاقتناص الفرص' : 'معطل'}
              </p>
              {status.kill_switch_persistence_error && (
                <p className="text-xs text-rose-400 mt-1" role="alert">
                  Kill-switch persistence error: {status.kill_switch_persistence_error}
                </p>
              )}
              {status.kill_switch_marker_present && (
                <p className="text-xs text-amber-400 mt-1">
                  Durable kill-switch marker is present on disk
                </p>
              )}
            </div>
          </div>

          {/* Action Buttons */}
          <div className="flex items-center gap-3">
            {status.emergency_stopped ? (
              <button
                onClick={onResumeExecution}
                className="flex items-center gap-2 px-4 py-2 rounded-xl text-xs font-bold bg-emerald-500 text-slate-950 hover:bg-emerald-400 shadow-lg shadow-emerald-500/20 transition-all"
              >
                <Zap className="w-4 h-4" />
                استئناف المحرك (Resume Trading)
              </button>
            ) : (
              <button
                onClick={onEmergencyStop}
                className="flex items-center gap-2 px-4 py-2 rounded-xl text-xs font-bold bg-rose-500/10 text-rose-400 border border-rose-500/30 hover:bg-rose-500/20 transition-all"
              >
                <AlertOctagon className="w-4 h-4" />
                إيقاف طوارئ فوري (Emergency Stop)
              </button>
            )}
          </div>
        </div>

        {/* System telemetry grid */}
        {status.observability && (
          <div className="grid grid-cols-2 md:grid-cols-4 lg:grid-cols-6 gap-3 mt-4">
            <div className="p-3 rounded-xl bg-[#0d1424] border border-[#19243e]">
              <span className="text-[10px] font-bold uppercase text-slate-400">اشتراكات WS النشطة</span>
              <div className="text-sm font-extrabold font-mono text-cyan-400 mt-1">
                {status.observability.ws_subscriptions_active}
              </div>
            </div>
            <div className="p-3 rounded-xl bg-[#0d1424] border border-[#19243e]">
              <span className="text-[10px] font-bold uppercase text-slate-400">استهلاك الذاكرة RSS</span>
              <div className="text-sm font-extrabold font-mono text-slate-200 mt-1">
                {(status.observability.process_rss_bytes / 1024 / 1024).toFixed(1)} MB
              </div>
            </div>
            <div className="p-3 rounded-xl bg-[#0d1424] border border-[#19243e]">
              <span className="text-[10px] font-bold uppercase text-slate-400">استهلاك المعالج CPU</span>
              <div className="text-sm font-extrabold font-mono text-slate-200 mt-1">
                {status.observability.process_cpu_percent.toFixed(1)}%
              </div>
            </div>
            <div className="p-3 rounded-xl bg-[#0d1424] border border-[#19243e]">
              <span className="text-[10px] font-bold uppercase text-slate-400">فجوات الستريم (Slot Gaps)</span>
              <div className={`text-sm font-extrabold font-mono mt-1 ${status.observability.stream_gaps_total > 0 ? 'text-amber-400' : 'text-slate-400'}`}>
                {status.observability.stream_gaps_total}
              </div>
            </div>
            <div className="p-3 rounded-xl bg-[#0d1424] border border-[#19243e]">
              <span className="text-[10px] font-bold uppercase text-slate-400">مرفوضات الـ Stale</span>
              <div className="text-sm font-extrabold font-mono text-slate-400 mt-1">
                {status.observability.stale_rejections_total}
              </div>
            </div>
            <div className="p-3 rounded-xl bg-[#0d1424] border border-[#19243e]">
              <span className="text-[10px] font-bold uppercase text-slate-400">مدة التشغيل (Uptime)</span>
              <div className="text-sm font-extrabold font-mono text-slate-200 mt-1 flex items-center gap-1">
                <Clock className="w-3 h-3 text-slate-400" />
                {Math.floor(status.uptime_secs / 3600)}h {Math.floor((status.uptime_secs % 3600) / 60)}m
              </div>
            </div>
          </div>
        )}
      </div>
    </div>
  );
};

export default Overview;
