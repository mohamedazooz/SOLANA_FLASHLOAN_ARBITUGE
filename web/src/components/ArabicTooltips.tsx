/* ============================================================
   Arabic Tooltips & Explanations
   Reusable components for Arabic/English bilingual support
   ============================================================ */

import { useState } from "react";

/* ============================================================
   Arabic Content Dictionary
   ============================================================ */

export const ARABIC_CONTENT = {
  // Dashboard sections
  dashboard: {
    title: "لوحة تحكم البوت الموحدة",
    subtitle: "مراقبة مباشرة للفرص، المسابح، الأرباح، والسجلات",
  },
  
  // Bot modes
  modes: {
    live: { en: "Live", ar: "مباشر", desc: "تنفيذ حقيقي بأموال حقيقية" },
    shadow: { en: "Shadow", ar: "ظل", desc: "محاكاة باستخدام بيانات تاريخية" },
    paper: { en: "Paper", ar: "ورقي", desc: "تداول تجريبي بدون أموال حقيقية" },
    emergency: { en: "Emergency", ar: "طوارئ", desc: "إيقاف فوري لجميع العمليات" },
  },

  // Control buttons
  controls: {
    start: { en: "Start", ar: "بدء", desc: "بدء تنفيذ الصفقات" },
    stop: { en: "Stop", ar: "إيقاف", desc: "إيقاف تنفيذ الصفقات" },
    shadow: { en: "Shadow", ar: "وضع الظل", desc: "التبديل لوضع المحاكاة" },
    paper: { en: "Paper", ar: "وضع الورقي", desc: "التبديل لوضع التداول التجريبي" },
    live: { en: "Live", ar: "مباشر", desc: "التبديل للوضع المباشر (يتطلب تأكيد)" },
    emergency: { en: "Emergency", ar: "طوارئ", desc: "تفعيل إيقاف الطوارئ" },
  },

  // Stats
  stats: {
    mode: { en: "Mode", ar: "الوضع", desc: "وضع تشغيل البوت الحالي" },
    execution: { en: "Execution", ar: "التنفيذ", desc: "حالة تنفيذ الصفقات" },
    slot: { en: "Network Slot", ar: "فتحة الشبكة", desc: "رقم الفتحة الحالية في شبكة سولانا" },
    pools: { en: "Pools Tracked", ar: "المسابح المتتبعة", desc: "عدد المسابح التي يراقبها البوت" },
    opportunities: { en: "Opportunities", ar: "الفرص", desc: "إجمالي الفرص المكتشفة" },
    profitable: { en: "Profitable", ar: "مربحة", desc: "الفرص ذات الربح المتوقع الإيجابي" },
    executed: { en: "Executed", ar: "منفذة", desc: "عدد الصفقات المنفذة" },
    successful: { en: "Successful", ar: "ناجحة", desc: "الصفقات التي تمت بنجاح" },
    failed: { en: "Failed", ar: "فاشلة", desc: "الصفقات التي فشلت" },
    pnl: { en: "Net PnL", ar: "صافي الربح/الخسارة", desc: "إجمالي الربح أو الخسارة بالـ SOL" },
    uptime: { en: "Uptime", ar: "وقت التشغيل", desc: "مدة تشغيل البوت منذ آخر إعادة تشغيل" },
  },

  // Connections
  connections: {
    stream: { en: "Primary Stream", ar: "التدفق الأساسي", desc: "اتصال WebSocket الأساسي للبيانات" },
    helius: { en: "Helius", ar: "هيليوس", desc: "اتصال هيليوس للبيانات المحسنة" },
  },

  // Opportunities
  opportunities: {
    title: { en: "Live Opportunities", ar: "الفرص المباشرة" },
    time: { en: "Time", ar: "الوقت", desc: "وقت اكتشاف الفرصة" },
    slot: { en: "Slot", ar: "الفتحة", desc: "رقم الفتحة عند الاكتشاف" },
    route: { en: "Route", ar: "المسار", desc: "مسار التداول (مثل: SOL → USDC → SOL)" },
    protocol: { en: "Protocol", ar: "البروتوكول", desc: "بروتوكول DEX المستخدم" },
    borrow: { en: "Borrow", ar: "المقترض", desc: "مبلغ الفلاش لوان المقترض" },
    expected: { en: "Expected", ar: "المتوقع", desc: "المخرج المتوقع بعد المسار" },
    profit: { en: "Net Profit", ar: "صافي الربح", desc: "الربح الصافي بعد الرسوم" },
    status: { en: "Status", ar: "الحالة", desc: "هل الفرصة مؤهلة للتنفيذ" },
    eligible: { en: "Eligible", ar: "مؤهلة", desc: "تجتاز جميع فحوصات التحقق" },
    rejected: { en: "Rejected", ar: "مرفوضة", desc: "لم تجتاز فحوصات التحقق" },
    filterProtocol: { en: "Filter Protocol", ar: "تصفية البروتوكول", desc: "تصفية حسب اسم البروتوكول" },
    filterEligible: { en: "Eligible Only", ar: "مؤهلة فقط", desc: "إظهار الفرص المؤهلة فقط" },
    filterMinNet: { en: "Min Net Profit", ar: "أقل ربح صافي", desc: "الحد الأدنى للربح الصافي بالـ SOL" },
  },

  // Executions
  executions: {
    title: { en: "Executions", ar: "التنفيذات" },
    time: { en: "Time", ar: "الوقت" },
    outcome: { en: "Outcome", ar: "النتيجة", desc: "نتيجة التنفيذ" },
    landed: { en: "Landed", ar: "تمت", desc: "تم تنفيذ الصفقة بنجاح" },
    dropped: { en: "Dropped", ar: "أسقطت", desc: "لم يتم تنفيذ الصفقة" },
    simulated: { en: "Simulated", ar: "محاكاة", desc: "تمت محاكاة الصفقة" },
    route: { en: "Route", ar: "المسار" },
    detail: { en: "Detail", ar: "التفاصيل", desc: "تفاصيل إضافية عن التنفيذ" },
  },

  // Logs
  logs: {
    title: { en: "Live Logs", ar: "السجلات المباشرة" },
    time: { en: "Time", ar: "الوقت" },
    level: { en: "Level", ar: "المستوى", desc: "مستوى السجل" },
    info: { en: "Info", ar: "معلومات", desc: "معلومات عامة" },
    warn: { en: "Warning", ar: "تحذير", desc: "تحذير يتطلب انتباه" },
    error: { en: "Error", ar: "خطأ", desc: "خطأ يحتاج إصلاح" },
    debug: { en: "Debug", ar: "تصحيح", desc: "معلومات تصحيح" },
    success: { en: "Success", ar: "نجاح", desc: "عملية نجحت" },
    stage: { en: "Stage", ar: "المرحلة", desc: "مرحلة المعالجة" },
    message: { en: "Message", ar: "الرسالة", desc: "محتوى السجل" },
    filter: { en: "Filter Level", ar: "تصفية المستوى", desc: "تصفية السجلات حسب المستوى" },
  },

  // Pools
  pools: {
    title: { en: "Tracked Pools", ar: "المسابح المتتبعة" },
    pool: { en: "Pool", ar: "المسبح", desc: "معرف المسبح" },
    protocol: { en: "Protocol", ar: "البروتوكول" },
    pair: { en: "Pair", ar: "الزوج", desc: "زوج العملات في المسبح" },
    lastUpdate: { en: "Last Update", ar: "آخر تحديث", desc: "آخر فتحة تم تحديث المسبح فيها" },
    description: { en: "Pool inventory published by supervisor. Tracks Meteora DLMM, Raydium, Orca, Pump.fun, PumpSwap.", ar: "مخزون المسابح المنشور من المشرف. يتتبع Meteora DLMM، Raydium، Orca، Pump.fun، PumpSwap." },
  },

  // Protocols
  protocols: {
    title: { en: "Protocol Coverage", ar: "تغطية البروتوكولات" },
    protocol: { en: "Protocol", ar: "البروتوكول" },
    pools: { en: "Pools", ar: "المسابح", desc: "عدد المسابح في هذا البروتوكول" },
    share: { en: "Share", ar: "النسبة", desc: "نسبة المسابح من الإجمالي" },
    avgFee: { en: "Avg Fee (bps)", ar: "متوسط الرسوم (نقطة أساس)", desc: "متوسط رسوم البروتوكول" },
    connectivity: { en: "Connectivity", ar: "الاتصال", desc: "هل البروتوكول قابل للتوجيه" },
    routable: { en: "Routable", ar: "قابل للتوجيه", desc: "يمكن استخدامه في مسارات التحكيم" },
    noEdges: { en: "No Edges", ar: "بدون حواف", desc: "لا توجد مسارات متاحة" },
    description: { en: "Cross-protocol cycles discovered by unified graph path", ar: "دورات عبر البروتوكولات مكتشفة بمسار الرسم الموحد" },
  },

  // Terminal
  terminal: {
    title: { en: "Bot Terminal", ar: "طرفية البوت" },
    connected: { en: "Connected", ar: "متصل" },
    disconnected: { en: "Disconnected", ar: "غير متصل" },
    clear: { en: "Clear", ar: "مسح", desc: "مسح محتوى الطرفية" },
    prompt: { en: "Command", ar: "أمر", desc: "أدخل أمراً للتنفيذ" },
    help: { en: "Help", ar: "مساعدة", desc: "عرض الأوامر المتاحة" },
    commands: {
      help: { en: "Show available commands", ar: "عرض الأوامر المتاحة" },
      status: { en: "Show bot status and mode", ar: "عرض حالة البوت والوضع" },
      start: { en: "Start execution", ar: "بدء التنفيذ" },
      stop: { en: "Stop execution", ar: "إيقاف التنفيذ" },
      shadow: { en: "Switch to shadow mode", ar: "التبديل لوضع الظل" },
      paper: { en: "Switch to paper mode", ar: "التبديل لوضع الورقي" },
      live: { en: "Switch to live mode", ar: "التبديل للوضع المباشر" },
      emergency: { en: "Engage emergency stop", ar: "تفعيل إيقاف الطوارئ" },
      "clear-emergency": { en: "Clear emergency stop", ar: "إلغاء إيقاف الطوارئ" },
      logs: { en: "Show recent logs", ar: "عرض السجلات الأخيرة" },
      pools: { en: "List tracked pools", ar: "قائمة المسابح المتتبعة" },
      opportunities: { en: "Show live opportunities", ar: "عرض الفرص المباشرة" },
      executions: { en: "Show recent executions", ar: "عرض التنفيذات الأخيرة" },
      config: { en: "Show current configuration", ar: "عرض الإعدادات الحالية" },
      pnl: { en: "Show profit/loss summary", ar: "عرض ملخص الربح/الخسارة" },
      providers: { en: "Show provider status", ar: "عرض حالة المزودين" },
      clear: { en: "Clear terminal", ar: "مسح الطرفية" },
      exit: { en: "Disconnect terminal", ar: "قطع اتصال الطرفية" },
    },
  },

  // PnL
  pnl: {
    positive: { en: "Profit", ar: "ربح", desc: "صافي ربح إيجابي" },
    negative: { en: "Loss", ar: "خسارة", desc: "صافي خسارة" },
    neutral: { en: "Break-even", ar: "تعادل", desc: "لا ربح ولا خسارة" },
  },

  // General
  general: {
    loading: { en: "Loading...", ar: "جاري التحميل..." },
    noData: { en: "No data available", ar: "لا توجد بيانات متاحة" },
    refresh: { en: "Refresh", ar: "تحديث", desc: "تحديث البيانات" },
    settings: { en: "Settings", ar: "الإعدادات" },
    language: { en: "Language", ar: "اللغة" },
    english: { en: "English", ar: "الإنجليزية" },
    arabic: { en: "Arabic", ar: "العربية" },
    close: { en: "Close", ar: "إغلاق" },
    confirm: { en: "Confirm", ar: "تأكيد" },
    cancel: { en: "Cancel", ar: "إلغاء" },
    save: { en: "Save", ar: "حفظ" },
    apply: { en: "Apply", ar: "تطبيق" },
  },
};

/* ============================================================
   Tooltip Component with Arabic Support
   ============================================================ */

interface TooltipProps {
  children: React.ReactNode;
  content: string;
  contentAr?: string;
  position?: "top" | "bottom" | "left" | "right";
  showArabic?: boolean;
}

export function Tooltip({ 
  children, 
  content, 
  contentAr, 
  position = "top",
  showArabic = true 
}: TooltipProps) {
  const [isVisible, setIsVisible] = useState(false);

  const tooltipContent = showArabic && contentAr 
    ? `${content}\n\n${contentAr}`
    : content;

  return (
    <div 
      className="tooltip-wrapper"
      style={{ 
        position: "relative", 
        display: "inline-flex" 
      }}
      onMouseEnter={() => setIsVisible(true)}
      onMouseLeave={() => setIsVisible(false)}
      onFocus={() => setIsVisible(true)}
      onBlur={() => setIsVisible(false)}
    >
      {children}
      {isVisible && (
        <div 
          className="tooltip"
          style={{
            position: "absolute",
            zIndex: 1000,
            padding: "8px 12px",
            background: "var(--bg-elevated)",
            border: "1px solid var(--border-bright)",
            borderRadius: "8px",
            fontSize: "11px",
            color: "var(--text)",
            whiteSpace: "pre-wrap",
            maxWidth: "300px",
            boxShadow: "0 8px 24px rgba(0, 0, 0, 0.5)",
            pointerEvents: "none",
            ...getPositionStyles(position),
          }}
        >
          {tooltipContent.split("\n").map((line, i) => (
            <div key={i} style={{ 
              color: i === 0 ? "var(--text)" : "var(--text-muted)",
              fontWeight: i === 0 ? 600 : 400,
              marginBottom: i < tooltipContent.split("\n").length - 1 ? "4px" : 0,
            }}>
              {line}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

function getPositionStyles(position: string) {
  const base = { transform: "translateX(-50%)" };
  switch (position) {
    case "top":
      return { ...base, bottom: "100%", left: "50%", marginBottom: "8px" };
    case "bottom":
      return { ...base, top: "100%", left: "50%", marginTop: "8px" };
    case "left":
      return { ...base, right: "100%", top: "50%", marginRight: "8px", transform: "translateY(-50%)" };
    case "right":
      return { ...base, left: "100%", top: "50%", marginLeft: "8px", transform: "translateY(-50%)" };
    default:
      return { ...base, bottom: "100%", left: "50%", marginBottom: "8px" };
  }
}

/* ============================================================
   Arabic Toggle Button
   ============================================================ */

interface ArabicToggleProps {
  showArabic: boolean;
  onToggle: (show: boolean) => void;
}

export function ArabicToggle({ showArabic, onToggle }: ArabicToggleProps) {
  return (
    <button
      onClick={() => onToggle(!showArabic)}
      style={{
        display: "flex",
        alignItems: "center",
        gap: "6px",
        padding: "6px 12px",
        background: "var(--panel)",
        border: "1px solid var(--border)",
        borderRadius: "20px",
        color: "var(--text)",
        fontSize: "11px",
        fontWeight: 500,
        cursor: "pointer",
        transition: "all 0.2s",
      }}
      onMouseOver={(e) => { e.currentTarget.style.borderColor = "var(--border-bright)"; }}
      onMouseOut={(e) => { e.currentTarget.style.borderColor = "var(--border)"; }}
    >
      <span style={{ fontSize: "14px" }}>{showArabic ? "🇸🇦" : "🇺🇸"}</span>
      <span>{showArabic ? "العربية" : "English"}</span>
    </button>
  );
}

/* ============================================================
   Explanation Panel with Arabic
   ============================================================ */

interface ExplanationPanelProps {
  title: string;
  titleAr?: string;
  children: React.ReactNode;
  childrenAr?: React.ReactNode;
  showArabic?: boolean;
  icon?: string;
}

export function ExplanationPanel({ 
  title, 
  titleAr, 
  children, 
  childrenAr, 
  showArabic = true,
  icon = "ℹ️"
}: ExplanationPanelProps) {
  return (
    <div style={{
      background: "linear-gradient(135deg, rgba(0, 255, 136, 0.03) 0%, rgba(0, 212, 255, 0.03) 100%)",
      border: "1px solid var(--border)",
      borderLeft: "2px solid var(--emerald)",
      borderRadius: "10px",
      padding: "14px 18px",
      marginBottom: "16px",
      fontSize: "11px",
      lineHeight: "1.7",
    }}>
      <div style={{ 
        display: "flex", 
        alignItems: "center", 
        gap: "8px", 
        marginBottom: "10px",
        fontSize: "11px",
        fontWeight: 700,
        color: "var(--emerald)",
        textTransform: "uppercase",
        letterSpacing: "0.05em",
      }}>
        <span>{icon}</span>
        <span>{title}</span>
        {showArabic && titleAr && <span style={{ color: "var(--text-muted)", fontWeight: 400 }}>/ {titleAr}</span>}
      </div>
      <div style={{ color: "var(--text-muted)" }}>
        {children}
        {showArabic && childrenAr && (
          <div style={{ marginTop: "10px", paddingTop: "10px", borderTop: "1px solid var(--border)" }}>
            {childrenAr}
          </div>
        )}
      </div>
    </div>
  );
}

/* ============================================================
   Helper: Get Arabic Content
   ============================================================ */

export function getArabicContent(path: string): { en: string; ar: string } | null {
  const keys = path.split(".");
  let current: any = ARABIC_CONTENT;
  
  for (const key of keys) {
    if (current && typeof current === "object" && key in current) {
      current = current[key];
    } else {
      return null;
    }
  }
  
  if (current && typeof current === "object" && "en" in current && "ar" in current) {
    return { en: current.en, ar: current.ar };
  }
  
  return null;
}

/* ============================================================
   Bilingual Text Component
   ============================================================ */

interface BilingualTextProps {
  en: string;
  ar: string;
  showArabic: boolean;
  className?: string;
  style?: React.CSSProperties;
}

export function BilingualText({ en, ar, showArabic, className, style }: BilingualTextProps) {
  return (
    <span className={className} style={style}>
      {showArabic ? ar : en}
    </span>
  );
}