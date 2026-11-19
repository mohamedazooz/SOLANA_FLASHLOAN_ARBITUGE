import React, { useEffect, useState, useMemo } from 'react';
import {
  Waves,
  Search,
  ExternalLink,
  Copy,
  CheckCircle2,
  RefreshCw,
} from 'lucide-react';
import { api } from '../api';

interface PoolRow {
  pool_id: string;
  protocol: string;
  pair_name: string;
  base_mint: string;
  quote_mint: string;
  last_update_slot: number;
}

export const Pools: React.FC = () => {
  const [rows, setRows] = useState<PoolRow[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [search, setSearch] = useState('');
  const [protocolFilter, setProtocolFilter] = useState('');
  const [copiedId, setCopiedId] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  const loadPools = async () => {
    setLoading(true);
    try {
      const [status, opps] = await Promise.all([
        api.status(),
        api.opportunities().catch(() => []),
      ]);

      const poolMap = new Map<string, PoolRow>();
      for (const o of opps) {
        if (o.hop_quotes) {
          for (const h of o.hop_quotes) {
            if (h.pool && !poolMap.has(h.pool)) {
              let pair = 'SOL / USDC';
              if (h.protocol.toLowerCase().includes('pump')) pair = 'MEME / SOL';
              else if (h.pool.includes('usdt')) pair = 'SOL / USDT';

              poolMap.set(h.pool, {
                pool_id: h.pool,
                protocol: h.protocol || 'Raydium',
                pair_name: pair,
                base_mint: 'So11111111111111111111111111111111111111112',
                quote_mint: 'EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v',
                last_update_slot: o.slot || status.slot,
              });
            }
          }
        }
      }

      if (poolMap.size > 0) {
        setRows(Array.from(poolMap.values()));
      } else {
        // Fallback default high-volume pools tracked
        setRows([
          {
            pool_id: '58oQChx4yWmvKdwLLZzBi4ChoCc2fqCUWBkwMihLYQo2',
            protocol: 'Raydium V4',
            pair_name: 'SOL / USDC',
            base_mint: 'So11111111111111111111111111111111111111112',
            quote_mint: 'EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v',
            last_update_slot: status.slot,
          },
          {
            pool_id: '7qbRF6YsyGuLUVs6Y1q64bdVrfe4ZcUUz1JRdoVNUJnm',
            protocol: 'Raydium CLMM',
            pair_name: 'SOL / USDT',
            base_mint: 'So11111111111111111111111111111111111111112',
            quote_mint: 'Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB',
            last_update_slot: status.slot,
          },
          {
            pool_id: '2zMMhcVQEXDtdE6vsFS7S7D5oUodfJHE8vd1gnBouauv',
            protocol: 'Orca Whirlpool',
            pair_name: 'SOL / USDC',
            base_mint: 'So11111111111111111111111111111111111111112',
            quote_mint: 'EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v',
            last_update_slot: status.slot,
          },
          {
            pool_id: '5rCf1DM8LjKTw4YqhnoLcngyZYeNnQqztScTogYHAS6',
            protocol: 'Meteora DLMM',
            pair_name: 'SOL / USDC (Active Bin)',
            base_mint: 'So11111111111111111111111111111111111111112',
            quote_mint: 'EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v',
            last_update_slot: status.slot,
          },
        ]);
      }
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : 'failed to load pools');
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    loadPools();
    const interval = setInterval(loadPools, 3000);
    return () => clearInterval(interval);
  }, []);

  const handleCopy = (text: string) => {
    navigator.clipboard.writeText(text);
    setCopiedId(text);
    setTimeout(() => setCopiedId(null), 2000);
  };

  const filtered = useMemo(() => {
    return rows.filter((r) => {
      if (protocolFilter && !r.protocol.toLowerCase().includes(protocolFilter.toLowerCase())) return false;
      if (search.trim()) {
        const q = search.toLowerCase();
        return (
          r.pool_id.toLowerCase().includes(q) ||
          r.protocol.toLowerCase().includes(q) ||
          r.pair_name.toLowerCase().includes(q)
        );
      }
      return true;
    });
  }, [rows, protocolFilter, search]);

  return (
    <div className="space-y-4">
      {/* ── Header ── */}
      <div className="flex flex-wrap items-center justify-between gap-3 p-4 rounded-xl bg-[#0b101d] border border-[#172238] shadow-lg">
        <div className="flex items-center gap-3">
          <div className="p-2.5 rounded-lg bg-emerald-500/10 border border-emerald-500/20 text-emerald-400">
            <Waves className="w-6 h-6" />
          </div>
          <div>
            <h2 className="text-base font-bold text-slate-100 flex items-center gap-2">
              حلبة أحواض السيولة المتتبعة (Tracked Liquidity Pools)
              <span className="px-2 py-0.5 rounded-full text-[10px] font-mono font-bold bg-emerald-500/20 text-emerald-300 border border-emerald-500/30">
                {filtered.length} حوض نشط
              </span>
            </h2>
            <p className="text-xs text-slate-400">
              تتبع أحواض Meteora DLMM، Raydium AMM/CPMM/CLMM، Orca Whirlpool، و PumpSwap
            </p>
          </div>
        </div>

        <button
          onClick={loadPools}
          disabled={loading}
          className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-semibold bg-[#121a2d] text-slate-300 border border-[#1c2742] hover:bg-[#1a2540] transition-colors"
        >
          <RefreshCw className={`w-3.5 h-3.5 ${loading ? 'animate-spin' : ''}`} />
          تحديث
        </button>
      </div>

      {/* ── Search & Filter ── */}
      <div className="flex flex-wrap items-center gap-3 p-3 rounded-lg bg-[#0e1424] border border-[#19243d] text-xs">
        <div className="relative flex-1 min-w-[200px]">
          <Search className="w-4 h-4 text-slate-400 absolute right-3 top-1/2 -translate-y-1/2 pointer-events-none" />
          <input
            type="text"
            placeholder="بحث بعنوان الحوض، الزوج، أو البروتوكول..."
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            className="w-full pl-3 pr-9 py-1.5 rounded-lg bg-[#070b14] border border-[#1d2a47] text-slate-200 placeholder-slate-500 focus:outline-none focus:border-cyan-400"
          />
        </div>

        <select
          value={protocolFilter}
          onChange={(e) => setProtocolFilter(e.target.value)}
          className="px-3 py-1.5 rounded-lg bg-[#070b14] border border-[#1d2a47] text-slate-300 focus:outline-none focus:border-cyan-400"
        >
          <option value="">جميع البروتوكولات</option>
          <option value="Raydium">Raydium</option>
          <option value="CLMM">CLMM</option>
          <option value="Orca">Orca Whirlpool</option>
          <option value="Meteora">Meteora DLMM</option>
          <option value="Pump">Pump.fun</option>
        </select>
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
              <th className="p-3">عنوان الحوض (Pool Pubkey)</th>
              <th className="p-3">البروتوكول</th>
              <th className="p-3">زوج التداول</th>
              <th className="p-3">آخر تحديث (Slot)</th>
              <th className="p-3 text-center">مستكشف Solana</th>
            </tr>
          </thead>
          <tbody className="divide-y divide-[#131b2e]">
            {filtered.map((r, i) => {
              const isCopied = copiedId === r.pool_id;
              return (
                <tr key={i} className="hover:bg-[#0e1628] transition-colors">
                  <td className="p-3 font-mono font-semibold text-slate-200">
                    <div className="flex items-center gap-2">
                      <span className="bg-[#0f172a] px-2 py-0.5 rounded border border-[#1e293b] select-all">
                        {r.pool_id}
                      </span>
                      <button
                        onClick={() => handleCopy(r.pool_id)}
                        className="p-1 rounded text-slate-500 hover:text-slate-200 transition-colors"
                        title="نسخ عنوان الحوض"
                      >
                        {isCopied ? <CheckCircle2 className="w-3.5 h-3.5 text-emerald-400" /> : <Copy className="w-3.5 h-3.5" />}
                      </button>
                    </div>
                  </td>
                  <td className="p-3">
                    <span className="px-2 py-0.5 rounded text-[11px] font-bold bg-[#131c33] text-cyan-300 border border-[#1f2d4e]">
                      {r.protocol}
                    </span>
                  </td>
                  <td className="p-3 font-mono font-bold text-slate-200">
                    {r.pair_name}
                  </td>
                  <td className="p-3 font-mono text-cyan-400">
                    #{r.last_update_slot.toLocaleString()}
                  </td>
                  <td className="p-3 text-center">
                    <a
                      href={`https://solscan.io/account/${r.pool_id}`}
                      target="_blank"
                      rel="noreferrer"
                      className="inline-flex items-center gap-1 text-[11px] text-slate-400 hover:text-cyan-400 transition-colors"
                    >
                      <span>Solscan</span>
                      <ExternalLink className="w-3 h-3" />
                    </a>
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
    </div>
  );
};

export default Pools;
