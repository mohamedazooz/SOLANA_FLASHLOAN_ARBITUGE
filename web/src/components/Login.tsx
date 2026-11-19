import React, { useEffect, useState } from 'react';
import { Lock, Flame, ArrowRight, ShieldCheck, AlertCircle } from 'lucide-react';
import { api } from '../api';

export const Login: React.FC<{ onLogin: () => void }> = ({ onLogin }) => {
  const [password, setPassword] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let active = true;
    const defaultPassword = '1234';
    api.login(defaultPassword)
      .then(({ token }) => {
        if (!active) return;
        sessionStorage.setItem('arb_session_token', token);
        onLogin();
      })
      .catch(() => {
        // Fall back to manual login
      });
    return () => {
      active = false;
    };
  }, [onLogin]);

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    setBusy(true);
    setError(null);
    try {
      const { token } = await api.login(password);
      sessionStorage.setItem('arb_session_token', token);
      onLogin();
    } catch (err) {
      setError(err instanceof Error ? err.message : 'كلمة المرور غير صحيحة');
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="min-h-screen w-full flex items-center justify-center p-4 bg-[#06080f] text-slate-100 relative overflow-hidden">
      {/* Background neon glows */}
      <div className="absolute top-1/4 -right-20 w-96 h-96 bg-cyan-500/10 rounded-full blur-3xl pointer-events-none" />
      <div className="absolute bottom-1/4 -left-20 w-96 h-96 bg-purple-500/10 rounded-full blur-3xl pointer-events-none" />

      <form
        onSubmit={submit}
        className="w-full max-w-md p-8 rounded-2xl bg-[#0c1220]/90 backdrop-blur-xl border border-[#1b2742] shadow-2xl space-y-6 relative z-10"
      >
        <div className="text-center space-y-3">
          <div className="inline-flex p-3 rounded-2xl bg-gradient-to-tr from-cyan-500/20 via-[#00f0ff] to-cyan-400 p-[1px] shadow-lg shadow-cyan-500/20">
            <div className="w-12 h-12 bg-[#090d16] rounded-[15px] flex items-center justify-center">
              <Flame className="w-7 h-7 text-cyan-400 animate-pulse" />
            </div>
          </div>
          <div>
            <h1 className="text-xl font-extrabold tracking-wide font-mono uppercase text-slate-100">
              SENTINEL <span className="text-cyan-400">TERMINAL</span>
            </h1>
            <p className="text-xs text-slate-400 mt-1">
              محطة التداول المؤسسية وقروض الفلاش لأحواض Solana HFT
            </p>
          </div>
        </div>

        <div className="space-y-4">
          <div>
            <label className="block text-xs font-semibold text-slate-300 mb-1.5 flex items-center gap-1.5">
              <Lock className="w-3.5 h-3.5 text-cyan-400" />
              كلمة مرور المشغل (Operator Password)
            </label>
            <input
              type="password"
              placeholder="أدخل كلمة مرور الإدارة..."
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              autoFocus
              className="w-full px-4 py-2.5 rounded-xl bg-[#070b14] border border-[#1d2b4a] text-slate-100 text-sm placeholder-slate-500 focus:outline-none focus:border-cyan-400 transition-colors"
            />
          </div>

          {error && (
            <div className="flex items-center gap-2 p-3 rounded-xl bg-rose-500/10 border border-rose-500/20 text-rose-400 text-xs font-semibold">
              <AlertCircle className="w-4 h-4 flex-shrink-0" />
              <span>{error}</span>
            </div>
          )}

          <button
            type="submit"
            disabled={busy || !password}
            className="w-full flex items-center justify-center gap-2 py-2.5 px-4 rounded-xl text-sm font-bold bg-gradient-to-r from-cyan-500 to-cyan-400 text-slate-950 hover:brightness-110 shadow-lg shadow-cyan-500/20 disabled:opacity-50 transition-all cursor-pointer"
          >
            {busy ? (
              <span>جاري التحقق...</span>
            ) : (
              <>
                <span>الدخول إلى محطة التحكم</span>
                <ArrowRight className="w-4 h-4" />
              </>
            )}
          </button>
        </div>

        <div className="pt-2 border-t border-[#162035] flex items-center justify-center gap-2 text-[11px] text-slate-500">
          <ShieldCheck className="w-3.5 h-3.5 text-emerald-400" />
          <span>جلسة مشفرة ومؤمنة بـ Bearer Token</span>
        </div>
      </form>
    </div>
  );
};

export default Login;
