import { useEffect, useState, useMemo } from 'react';
import {
  Radar,
  CheckCircle2,
  XCircle,
  ArrowRight,
  TrendingUp,
  RefreshCw,
  Search,
} from 'lucide-react';
import { api, lamportsToSol, type OpportunityRecord } from '../api';

function shortAddress(addr: string) {
  return addr.length > 10 ? `${addr.slice(0, 4)}…${addr.slice(-4)}` : addr;
}

export const Opportunities: React.FC = () => {
  const [rows, setRows] = useState<OpportunityRecord[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [protocolFilter, setProtocolFilter] = useState('');
  const [searchQuery, setSearchQuery] = useState('');
  const [onlyEligible, setOnlyEligible] = useState(false);
  const [minNetSol, setMinNetSol] = useState('');
  const [loading, setLoading] = useState(false);

  const loadData = () => {
    setLoading(true);
    api.opportunities()
      .then((data) => {
        if (Array.isArray(data)) setRows(data);
        setError(null);
      })
      .catch((e) => setError(e.message))
      .finally(() => setLoading(false));
  };

  useEffect(() => {
    loadData();
    const timer = setInterval(loadData, 2500);
    return () => clearInterval(timer);
  }, []);

  const filtered = useMemo(() => {
    return rows.filter((r) => {
      if (protocolFilter && !r.protocol.toLowerCase().includes(protocolFilter.toLowerCase())) return false;
      if (onlyEligible && !r.eligible) return false;
      if (minNetSol && r.net_profit_lamports < Number(minNetSol) * 1e9) return false;
      if (searchQuery.trim()) {
        const q = searchQuery.toLowerCase();
        return (
          r.route.toLowerCase().includes(q) ||
          r.protocol.toLowerCase().includes(q) ||
          (r.reject_reason && r.reject_reason.toLowerCase().includes(q))
        );
      }
      return true;
    });
  }, [rows, protocolFilter, onlyEligible, minNetSol, searchQuery]);

  return (
    <div className="space-y-4">
      {/* ── Header & Quick Filters ── */}
      <div className="flex flex-wrap items-center justify-between gap-3 p-4 rounded-xl bg-[#0b101d] border border-[#172238] shadow-lg">
        <div className="flex items-center gap-3">
          <div className="p-2.5 rounded-lg bg-cyan-500/10 border border-cyan-500/20 text-cyan-400">
            <Radar className="w-6 h-6 animate-spin" />
          </div>
          <div>
            <h2 className="text-base font-bold text-slate-100 flex items-center gap-2">
              رادار الفرص الحية (Live Opportunity Radar)
              <span className="px-2 py-0.5 rounded-full text-[10px] font-mono font-bold bg-cyan-500/20 text-cyan-300 border border-cyan-500/30">
                {filtered.length} فرصة
              </span>
            </h2>
            <p className="text-xs text-slate-400">
              اكتشاف الفوارق السعرية عبر أحواض Raydium, Orca, Whirlpool, Meteora, و Pump.fun
            </p>
          </div>
        </div>

        <button
          onClick={loadData}
          disabled={loading}
          className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-semibold bg-[#121a2d] text-slate-300 border border-[#1c2742] hover:bg-[#1a2540] transition-colors"
        >
          <RefreshCw className={`w-3.5 h-3.5 ${loading ? 'animate-spin' : ''}`} />
          تحديث
        </button>
      </div>

      {/* ── Filters Toolbar ── */}
      <div className="flex flex-wrap items-center gap-3 p-3 rounded-lg bg-[#0e1424] border border-[#19243d] text-xs">
        <div className="relative flex-1 min-w-[200px]">
          <Search className="w-4 h-4 text-slate-400 absolute right-3 top-1/2 -translate-y-1/2 pointer-events-none" />
          <input
            type="text"
            placeholder="بحث بالمسار، التوكن، أو سبب الرفض..."
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
            className="w-full pl-3 pr-9 py-1.5 rounded-lg bg-[#070b14] border border-[#1d2a47] text-slate-200 placeholder-slate-500 focus:outline-none focus:border-cyan-400"
          />
        </div>

        <select
          value={protocolFilter}
          onChange={(e) => setProtocolFilter(e.target.value)}
          className="px-3 py-1.5 rounded-lg bg-[#070b14] border border-[#1d2a47] text-slate-300 focus:outline-none focus:border-cyan-400"
        >
          <option value="">جميع البروتوكولات</option>
          <option value="Raydium">Raydium V4</option>
          <option value="CLMM">Raydium CLMM</option>
          <option value="Orca">Orca Whirlpool</option>
          <option value="Meteora">Meteora DLMM</option>
          <option value="Pump">Pump.fun</option>
        </select>

        <label className="flex items-center gap-2 cursor-pointer text-slate-300 select-none">
          <input
            type="checkbox"
            checked={onlyEligible}
            onChange={(e) => setOnlyEligible(e.target.checked)}
            className="rounded border-[#1d2a47] text-cyan-500 focus:ring-0"
          />
          <span>المؤهلة فقط (Eligible)</span>
        </label>

        <input
          type="number"
          step="0.001"
          placeholder="أقل ربح صافٍ (SOL)..."
          value={minNetSol}
          onChange={(e) => setMinNetSol(e.target.value)}
          className="w-36 px-3 py-1.5 rounded-lg bg-[#070b14] border border-[#1d2a47] text-slate-300 placeholder-slate-500 focus:outline-none focus:border-cyan-400"
        />
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
              <th className="p-3">Slot</th>
              <th className="p-3">المسار (Route)</th>
              <th className="p-3">البروتوكول</th>
              <th className="p-3">قرض الفلاش (Borrow)</th>
              <th className="p-3">العائد المتوقع</th>
              <th className="p-3">صافي الربح (SOL)</th>
              <th className="p-3">تفاصيل القفزات (Hops)</th>
              <th className="p-3 text-center">الحالة</th>
            </tr>
          </thead>
          <tbody className="divide-y divide-[#131b2e]">
            {filtered.length === 0 ? (
              <tr>
                <td colSpan={9} className="p-8 text-center text-slate-500 font-mono">
                  لا توجد فرص مطابقة حالياً في هذه النافذة الزمنية
                </td>
              </tr>
            ) : (
              filtered.map((r, i) => {
                const isNetPositive = r.net_profit_lamports > 0;
                return (
                  <tr key={i} className="hover:bg-[#0e1628] transition-colors">
                    <td className="p-3 font-mono text-slate-400 whitespace-nowrap">
                      {new Date(r.captured_at_unix * 1000).toISOString().slice(11, 19)}
                    </td>
                    <td className="p-3 font-mono text-cyan-400 whitespace-nowrap">
                      #{r.slot.toLocaleString()}
                    </td>
                    <td className="p-3 font-mono font-semibold text-slate-200">
                      <span className="px-2 py-0.5 rounded bg-[#101728] border border-[#1b2640]">
                        {r.route}
                      </span>
                    </td>
                    <td className="p-3">
                      <span className="text-slate-300 font-semibold">{r.protocol}</span>
                    </td>
                    <td className="p-3 font-mono text-slate-300">
                      {lamportsToSol(r.borrow_lamports)} SOL
                    </td>
                    <td className="p-3 font-mono text-slate-300">
                      {lamportsToSol(r.expected_output_lamports)} SOL
                    </td>
                    <td className="p-3 font-mono font-bold whitespace-nowrap">
                      <span className={`flex items-center gap-1 ${isNetPositive ? 'text-emerald-400' : 'text-slate-400'}`}>
                        {isNetPositive && <TrendingUp className="w-3.5 h-3.5" />}
                        {lamportsToSol(r.net_profit_lamports)} SOL
                      </span>
                    </td>
                    <td className="p-3">
                      {r.hop_quotes && r.hop_quotes.length > 0 ? (
                        <div className="space-y-1">
                          {r.hop_quotes.map((h, j) => (
                            <div
                              key={j}
                              className="flex items-center gap-1 font-mono text-[11px] text-slate-400"
                            >
                              <span className="text-cyan-400">{h.protocol}:</span>
                              <span>{shortAddress(h.pool)}</span>
                              <ArrowRight className="w-3 h-3 text-slate-600" />
                              <span className="text-emerald-400 font-semibold">
                                {lamportsToSol(h.amount_out)}
                              </span>
                            </div>
                          ))}
                        </div>
                      ) : (
                        <span className="text-slate-600">—</span>
                      )}
                    </td>
                    <td className="p-3 text-center whitespace-nowrap">
                      {r.eligible ? (
                        <span className="inline-flex items-center gap-1 px-2.5 py-1 rounded-full text-[10px] font-mono font-bold bg-emerald-500/20 text-emerald-400 border border-emerald-500/30">
                          <CheckCircle2 className="w-3 h-3" /> مؤهلة للتنفيذ
                        </span>
                      ) : (
                        <span
                          title={r.reject_reason ?? 'غير مؤهلة'}
                          className="inline-flex items-center gap-1 px-2.5 py-1 rounded-full text-[10px] font-mono font-bold bg-rose-500/10 text-rose-400 border border-rose-500/20"
                        >
                          <XCircle className="w-3 h-3" /> {r.reject_reason ? r.reject_reason.slice(0, 15) : 'مرفوضة'}
                        </span>
                      )}
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

export default Opportunities;
