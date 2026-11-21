import React, { useState, useEffect, useRef, useMemo } from 'react';
import {
  Terminal,
  Search,
  Filter,
  Play,
  Pause,
  Trash2,
  Copy,
  ArrowDownCircle,
  CheckCircle2,
  AlertTriangle,
  XCircle,
  Info,
} from 'lucide-react';
import { api, type LogRecord } from '../api';

export const Logs: React.FC = () => {
  const [logs, setLogs] = useState<LogRecord[]>([]);
  const [filterLevel, setFilterLevel] = useState<string>('ALL');
  const [searchQuery, setSearchQuery] = useState<string>('');
  const [isPaused, setIsPaused] = useState<boolean>(false);
  const [autoScroll, setAutoScroll] = useState<boolean>(true);
  const [copied, setCopied] = useState<boolean>(false);
  const logsEndRef = useRef<HTMLDivElement>(null);
  const scrollContainerRef = useRef<HTMLDivElement>(null);

  // Initial load
  useEffect(() => {
    api.logs()
      .then((initial) => {
        if (Array.isArray(initial)) {
          setLogs(initial.slice(-500));
        }
      })
      .catch((err) => console.error('Failed to fetch initial logs', err));
  }, []);

  // WebSocket Live Stream
  useEffect(() => {
    let ws: WebSocket | null = null;
    let reconnectTimeout: any = null;

    let disposed = false;

    const connectWs = async () => {
      try {
        const wsUrl = await api.wsUrl();
        if (disposed) return;
        const socket = new WebSocket(wsUrl);
        ws = socket;

        socket.onmessage = (event) => {
          if (isPaused) return;
          try {
            const data = JSON.parse(event.data);
            if (data.type === 'log' || data.level) {
              const newLog: LogRecord = {
                at_unix: data.at_unix || Math.floor(Date.now() / 1000),
                level: (data.level || 'INFO').toUpperCase(),
                stage: data.stage || 'SYSTEM',
                message: data.message || JSON.stringify(data),
              };
              setLogs((prev) => {
                const next = [...prev, newLog];
                return next.length > 800 ? next.slice(-800) : next;
              });
            }
          } catch {
            // Raw text log fallback
            const newLog: LogRecord = {
              at_unix: Math.floor(Date.now() / 1000),
              level: 'INFO',
              stage: 'RAW',
              message: String(event.data),
            };
            setLogs((prev) => [...prev.slice(-800), newLog]);
          }
        };

        socket.onclose = () => {
          if (!disposed) reconnectTimeout = window.setTimeout(() => void connectWs(), 3000);
        };

        socket.onerror = () => {
          socket.close();
        };
      } catch {
        if (!disposed) reconnectTimeout = window.setTimeout(() => void connectWs(), 5000);
      }
    };

    void connectWs();

    return () => {
      disposed = true;
      if (ws) ws.close();
      if (reconnectTimeout !== null) window.clearTimeout(reconnectTimeout);
    };
  }, [isPaused]);

  // Auto-scroll when new logs arrive
  useEffect(() => {
    if (autoScroll && logsEndRef.current) {
      logsEndRef.current.scrollIntoView({ behavior: 'smooth' });
    }
  }, [logs, autoScroll]);

  const filteredLogs = useMemo(() => {
    return logs.filter((log) => {
      if (filterLevel !== 'ALL' && log.level !== filterLevel) {
        return false;
      }
      if (searchQuery.trim()) {
        const query = searchQuery.toLowerCase();
        return (
          log.message.toLowerCase().includes(query) ||
          log.stage.toLowerCase().includes(query) ||
          log.level.toLowerCase().includes(query)
        );
      }
      return true;
    });
  }, [logs, filterLevel, searchQuery]);

  const handleCopyLogs = () => {
    const text = filteredLogs
      .map(
        (l) =>
          `[${new Date(l.at_unix * 1000).toISOString()}] [${l.level}] [${l.stage}] ${l.message}`
      )
      .join('\n');
    navigator.clipboard.writeText(text);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  };

  const getLevelBadge = (level: string) => {
    switch (level) {
      case 'ERROR':
        return (
          <span className="flex items-center gap-1 px-2 py-0.5 rounded text-[10px] font-mono font-bold bg-rose-500/20 text-rose-400 border border-rose-500/30">
            <XCircle className="w-3 h-3" /> ERROR
          </span>
        );
      case 'WARN':
        return (
          <span className="flex items-center gap-1 px-2 py-0.5 rounded text-[10px] font-mono font-bold bg-amber-500/20 text-amber-400 border border-amber-500/30">
            <AlertTriangle className="w-3 h-3" /> WARN
          </span>
        );
      case 'DEBUG':
        return (
          <span className="flex items-center gap-1 px-2 py-0.5 rounded text-[10px] font-mono font-bold bg-purple-500/20 text-purple-400 border border-purple-500/30">
            <Terminal className="w-3 h-3" /> DEBUG
          </span>
        );
      default:
        return (
          <span className="flex items-center gap-1 px-2 py-0.5 rounded text-[10px] font-mono font-bold bg-cyan-500/20 text-cyan-400 border border-cyan-500/30">
            <Info className="w-3 h-3" /> INFO
          </span>
        );
    }
  };

  return (
    <div className="space-y-4">
      {/* ── Header & Toolbar ── */}
      <div className="flex flex-wrap items-center justify-between gap-3 p-4 rounded-xl bg-[#0b101d] border border-[#172238] shadow-lg">
        <div className="flex items-center gap-3">
          <div className="p-2.5 rounded-lg bg-cyan-500/10 border border-cyan-500/20 text-cyan-400">
            <Terminal className="w-6 h-6" />
          </div>
          <div>
            <h2 className="text-base font-bold text-slate-100 flex items-center gap-2">
              بث اللوجات المباشر (HFT Live Stream)
              <span className="flex h-2 w-2 relative">
                <span className={`animate-ping absolute inline-flex h-full w-full rounded-full ${isPaused ? 'bg-amber-400' : 'bg-emerald-400'} opacity-75`} />
                <span className={`relative inline-flex rounded-full h-2 w-2 ${isPaused ? 'bg-amber-500' : 'bg-emerald-500'}`} />
              </span>
            </h2>
            <p className="text-xs text-slate-400">
              متابعة تدفق الأوامر، فحص الأحواض، توقيعات Jito، واكتشاف الفرص بالمللي ثانية
            </p>
          </div>
        </div>

        {/* ── Actions ── */}
        <div className="flex items-center gap-2">
          <button
            onClick={() => setIsPaused(!isPaused)}
            className={`flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-semibold border transition-all ${
              isPaused
                ? 'bg-amber-500/20 text-amber-300 border-amber-500/40 hover:bg-amber-500/30'
                : 'bg-[#121a2d] text-slate-300 border-[#1c2742] hover:bg-[#1a2540]'
            }`}
          >
            {isPaused ? <Play className="w-3.5 h-3.5" /> : <Pause className="w-3.5 h-3.5" />}
            {isPaused ? 'استئناف البث' : 'إيقاف مؤقت'}
          </button>

          <button
            onClick={() => setAutoScroll(!autoScroll)}
            className={`flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-semibold border transition-all ${
              autoScroll
                ? 'bg-cyan-500/20 text-cyan-300 border-cyan-500/40'
                : 'bg-[#121a2d] text-slate-400 border-[#1c2742]'
            }`}
          >
            <ArrowDownCircle className="w-3.5 h-3.5" />
            تثبيت التمرير
          </button>

          <button
            onClick={handleCopyLogs}
            className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-semibold bg-[#121a2d] text-slate-300 border border-[#1c2742] hover:bg-[#1a2540] transition-colors"
          >
            {copied ? <CheckCircle2 className="w-3.5 h-3.5 text-emerald-400" /> : <Copy className="w-3.5 h-3.5" />}
            {copied ? 'تم النسخ!' : 'نسخ'}
          </button>

          <button
            onClick={() => setLogs([])}
            className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-semibold bg-rose-500/10 text-rose-400 border border-rose-500/20 hover:bg-rose-500/20 transition-colors"
          >
            <Trash2 className="w-3.5 h-3.5" />
            تفريغ
          </button>
        </div>
      </div>

      {/* ── Filters Bar ── */}
      <div className="flex flex-wrap items-center justify-between gap-3 p-3 rounded-lg bg-[#0e1424] border border-[#19243d]">
        <div className="flex items-center gap-2 flex-1 max-w-md">
          <div className="relative w-full">
            <Search className="w-4 h-4 text-slate-400 absolute right-3 top-1/2 -translate-y-1/2 pointer-events-none" />
            <input
              type="text"
              placeholder="بحث في رسائل اللوج، المراحل، أو المسارات..."
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
              className="w-full pl-3 pr-9 py-1.5 rounded-lg bg-[#070b14] border border-[#1d2a47] text-xs text-slate-200 placeholder-slate-500 focus:outline-none focus:border-cyan-400"
            />
          </div>
        </div>

        {/* Severity level buttons */}
        <div className="flex items-center gap-1.5">
          <Filter className="w-3.5 h-3.5 text-slate-400 ml-1" />
          {['ALL', 'INFO', 'WARN', 'ERROR', 'DEBUG'].map((level) => (
            <button
              key={level}
              onClick={() => setFilterLevel(level)}
              className={`px-2.5 py-1 rounded text-xs font-mono font-semibold transition-colors ${
                filterLevel === level
                  ? 'bg-cyan-500 text-slate-950 shadow-md shadow-cyan-500/20'
                  : 'bg-[#090e1a] text-slate-400 hover:text-slate-200 border border-[#1a2540]'
              }`}
            >
              {level}
            </button>
          ))}
          <span className="text-xs font-mono text-slate-400 mr-2">
            ({filteredLogs.length} سطر)
          </span>
        </div>
      </div>

      {/* ── Logs Terminal Output Screen ── */}
      <div
        ref={scrollContainerRef}
        className="h-[600px] overflow-y-auto rounded-xl bg-[#05070e] border border-[#162035] p-3 font-mono text-xs shadow-inner"
      >
        {filteredLogs.length === 0 ? (
          <div className="flex flex-col items-center justify-center h-full text-slate-500 space-y-2">
            <Terminal className="w-10 h-10 text-slate-600 animate-pulse" />
            <p>لا توجد سجلات مطابقة للفلتر المحدد في الوقت الحالي</p>
          </div>
        ) : (
          <div className="space-y-1">
            {filteredLogs.map((log, idx) => (
              <div
                key={idx}
                className="flex items-start gap-2.5 px-2.5 py-1.5 rounded hover:bg-[#0c1222] transition-colors border-b border-slate-900/40"
              >
                <span className="text-[11px] text-slate-500 select-none whitespace-nowrap">
                  {new Date(log.at_unix * 1000).toLocaleTimeString('en-US', {
                    hour12: false,
                    hour: '2-digit',
                    minute: '2-digit',
                    second: '2-digit',
                  })}
                </span>
                <div className="flex-shrink-0">{getLevelBadge(log.level)}</div>
                <span className="text-[11px] font-bold text-cyan-400/90 bg-cyan-950/40 px-1.5 py-0.5 rounded border border-cyan-800/30 whitespace-nowrap">
                  [{log.stage}]
                </span>
                <span className="text-slate-200 break-all leading-relaxed select-text flex-1">
                  {log.message}
                </span>
              </div>
            ))}
            <div ref={logsEndRef} />
          </div>
        )}
      </div>
    </div>
  );
};
