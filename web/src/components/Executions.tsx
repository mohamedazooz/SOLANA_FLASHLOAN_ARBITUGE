import React, { useEffect, useState } from 'react';
import {
  Zap,
  CheckCircle2,
  XCircle,
  ExternalLink,
  RefreshCw,
} from 'lucide-react';
import { api, type ExecutionRecord } from '../api';

export const Executions: React.FC = () => {
  const [rows, setRows] = useState<ExecutionRecord[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  const loadExecutions = () => {
    setLoading(true);
    api.executions()
      .then((data) => {
        if (Array.isArray(data)) setRows(data);
        setError(null);
      })
      .catch((e) => setError(e.message))
      .finally(() => setLoading(false));
  };

  useEffect(() => {
    loadExecutions();
    const interval = setInterval(loadExecutions, 3000);
    return () => clearInterval(interval);
  }, []);

  return (
    <div className="space-y-4">
      {/* ── Header ── */}
      <div className="flex flex-wrap items-center justify-between gap-3 p-4 rounded-xl bg-[#0b101d] border border-[#172238] shadow-lg">
        <div className="flex items-center gap-3">
          <div className="p-2.5 rounded-lg bg-cyan-500/10 border border-cyan-500/20 text-cyan-400">
            <Zap className="w-6 h-6" />
          </div>
          <div>
            <h2 className="text-base font-bold text-slate-100 flex items-center gap-2">
              سجل الصفقات المنفذة (Execution & Bundles Ledger)
              <span className="px-2 py-0.5 rounded-full text-[10px] font-mono font-bold bg-cyan-500/20 text-cyan-300 border border-cyan-500/30">
                {rows.length} عملية
              </span>
            </h2>
            <p className="text-xs text-slate-400">
              تتبع الصفقات المحققة على Mainnet، حزم Jito، استهلاك وحدات الحوسبة (CU)، وحجم المعاملة
            </p>
          </div>
        </div>

        <button
          onClick={loadExecutions}
          disabled={loading}
          className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-semibold bg-[#121a2d] text-slate-300 border border-[#1c2742] hover:bg-[#1a2540] transition-colors"
        >
          <RefreshCw className={`w-3.5 h-3.5 ${loading ? 'animate-spin' : ''}`} />
          تحديث
        </button>
      </div>

      {error && (
        <div className="p-3 rounded-lg bg-rose-500/10 border border-rose-500/20 text-rose-400 text-xs font-mono">
          {error}
        </div>
      )}

      {/* ── Table Container ── */}
      <div className="overflow-x-auto rounded-xl bg-[#090d18] border border-[#18233d] shadow-lg">
        <table className="w-full text-right text-xs">
          <thead>
            <tr className="border-b border-[#172238] bg-[#0c1220] text-slate-400 font-bold uppercase tracking-wider">
              <th className="p-3">الوقت (UTC)</th>
              <th className="p-3 text-center">النتيجة</th>
              <th className="p-3">المسار (Route)</th>
              <th className="p-3">التوقيع / الحزمة (Signature)</th>
              <th className="p-3">Slot</th>
              <th className="p-3 text-center">المحاكاة</th>
              <th className="p-3">حجم الـ Tx</th>
              <th className="p-3">وحدات الحوسبة (CU)</th>
              <th className="p-3">صافي الربح (SOL)</th>
              <th className="p-3">تفاصيل التنفيذ</th>
            </tr>
          </thead>
          <tbody className="divide-y divide-[#131b2e]">
            {rows.length === 0 ? (
              <tr>
                <td colSpan={10} className="p-8 text-center text-slate-500 font-mono">
                  لا توجد صفقات منفذة حتى الآن في هذه الجلسة (الوضع التجريبي والمحاكاة يعرض التوقيعات الحية فور هبوطها).
                </td>
              </tr>
            ) : (
              rows.map((r, i) => {
                const isSuccess = r.outcome === 'landed';
                return (
                  <tr key={i} className="hover:bg-[#0e1628] transition-colors">
                    <td className="p-3 font-mono text-slate-400 whitespace-nowrap">
                      {new Date(r.at_unix * 1000).toISOString().slice(11, 19)}
                    </td>
                    <td className="p-3 text-center whitespace-nowrap">
                      <span
                        className={`inline-flex items-center gap-1 px-2.5 py-0.5 rounded-full text-[10px] font-mono font-bold ${
                          isSuccess
                            ? 'bg-emerald-500/20 text-emerald-400 border border-emerald-500/30'
                            : 'bg-rose-500/20 text-rose-400 border border-rose-500/30'
                        }`}
                      >
                        {isSuccess ? <CheckCircle2 className="w-3 h-3" /> : <XCircle className="w-3 h-3" />}
                        {r.outcome}
                      </span>
                    </td>
                    <td className="p-3 font-mono font-semibold text-slate-200">
                      <span className="px-2 py-0.5 rounded bg-[#101728] border border-[#1b2640]">
                        {r.route}
                      </span>
                    </td>
                    <td className="p-3 font-mono whitespace-nowrap">
                      {r.signature ? (
                        <a
                          href={`https://solscan.io/tx/${r.signature}`}
                          target="_blank"
                          rel="noreferrer"
                          className="inline-flex items-center gap-1 text-cyan-400 hover:text-cyan-300 transition-colors"
                        >
                          <span>{r.signature.slice(0, 8)}…{r.signature.slice(-6)}</span>
                          <ExternalLink className="w-3 h-3" />
                        </a>
                      ) : (
                        <span className="text-slate-600">—</span>
                      )}
                    </td>
                    <td className="p-3 font-mono text-cyan-400">
                      {r.slot ? `#${r.slot.toLocaleString()}` : '—'}
                    </td>
                    <td className="p-3 text-center">
                      {r.sim_ok != null ? (
                        <span
                          className={`px-2 py-0.5 rounded text-[10px] font-mono font-bold ${
                            r.sim_ok
                              ? 'bg-emerald-500/10 text-emerald-400'
                              : 'bg-rose-500/10 text-rose-400'
                          }`}
                        >
                          {r.sim_ok ? 'PASS' : 'FAIL'}
                        </span>
                      ) : (
                        <span className="text-slate-600">—</span>
                      )}
                    </td>
                    <td className="p-3 font-mono text-slate-300">
                      {r.tx_size_bytes ? `${r.tx_size_bytes} B` : '—'}
                    </td>
                    <td className="p-3 font-mono text-slate-300">
                      {r.cu_consumed ? `${r.cu_consumed.toLocaleString()} CU` : '—'}
                    </td>
                    <td className="p-3 font-mono font-bold whitespace-nowrap">
                      {r.net_profit_sol != null ? (
                        <span className={r.net_profit_sol >= 0 ? 'text-emerald-400' : 'text-rose-400'}>
                          {r.net_profit_sol > 0 ? `+${r.net_profit_sol.toFixed(5)}` : r.net_profit_sol.toFixed(5)} SOL
                        </span>
                      ) : (
                        <span className="text-slate-600">—</span>
                      )}
                    </td>
                    <td className="p-3 text-slate-300 text-[11px] max-w-xs truncate" title={r.detail}>
                      {r.detail}
                    </td>
                  </tr>
                );
              })
            )}
          </tbody>
        </table>
      </div>
    </div>
  );
};

export default Executions;
