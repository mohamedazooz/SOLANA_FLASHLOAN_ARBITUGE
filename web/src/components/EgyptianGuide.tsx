import { useState } from "react";

export default function EgyptianGuide() {
  const [open, setOpen] = useState(true);

  return (
    <div className="card" style={{ border: "1px solid var(--accent-cyan, #00f3ff)", marginBottom: "1.25rem", background: "rgba(13, 18, 29, 0.95)" }}>
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: open ? "0.75rem" : "0" }}>
        <h3 style={{ color: "#00f3ff", margin: 0, display: "flex", alignItems: "center", gap: "0.5rem" }}>
          <span>💡</span> دليل البوت السريع (شرح باللهجة المصرية البسيطة لكل حاجة)
        </h3>
        <button
          onClick={() => setOpen(!open)}
          style={{
            background: "transparent",
            border: "1px solid #475569",
            color: "#94a3b8",
            padding: "0.25rem 0.6rem",
            borderRadius: "4px",
            cursor: "pointer",
            fontSize: "0.75rem",
          }}
        >
          {open ? "إخفاء الشرح ▲" : "إظهار الشرح ▼"}
        </button>
      </div>

      {open && (
        <div style={{ display: "grid", gridTemplateColumns: "repeat(auto-fit, minmax(260px, 1fr))", gap: "0.85rem", fontSize: "0.85rem", lineHeight: 1.6, color: "#cbd5e1" }}>
          <div style={{ background: "rgba(6, 8, 13, 0.6)", padding: "0.75rem 1rem", borderRadius: "6px", border: "1px solid rgba(255,255,255,0.05)" }}>
            <strong style={{ color: "#00f3ff", display: "block", marginBottom: "0.3rem" }}>1️⃣ يعني إيه فلاش لون (Flashloan)؟</strong>
            ده قرض لحظي بتاخده من بروتوكول كامينو (Kamino) في جزء من الألف من الثانية، من غير أي ضمان أو رهن ومن غير ما تدفع مليم من جيبك! البوت بينفذ الصفقة ويرجع القرض في نفس المعاملة (Atomic Tx). لو كسبت بتاخد الصافي، ولو منجحتش المعاملة بتتلغي ومبتخسرش مليم.
          </div>

          <div style={{ background: "rgba(6, 8, 13, 0.6)", padding: "0.75rem 1rem", borderRadius: "6px", border: "1px solid rgba(255,255,255,0.05)" }}>
            <strong style={{ color: "#00f3ff", display: "block", marginBottom: "0.3rem" }}>2️⃣ يعني إيه سبريد وفرق أسعار (Spread)؟</strong>
            السبريد هو فرق السعر بين منصة والتانية لنفس التوكن. مثلاً سعر التوكن على أوركا (Orca) أرخص من ميتيورا (Meteora). كل ما فرق السعر يزيد، كل ما صافي الربح يكبر، والبوت بيقتنص الفرص دي لحظياً.
          </div>

          <div style={{ background: "rgba(6, 8, 13, 0.6)", padding: "0.75rem 1rem", borderRadius: "6px", border: "1px solid rgba(255,255,255,0.05)" }}>
            <strong style={{ color: "#00f3ff", display: "block", marginBottom: "0.3rem" }}>3️⃣ خطوات المسار الأربعة</strong>
            1) بنستلف سولانا من كامينو فلاش لون ➔ 2) بنشتري التوكن بالرخيص من المنصة الأولى ➔ 3) بنبيع التوكن بالغالي في المنصة التانية ➔ 4) بنسدد قرض كامينو وإكرامية جيتو وصافي المكسب في محفظتك!
          </div>

          <div style={{ background: "rgba(6, 8, 13, 0.6)", padding: "0.75rem 1rem", borderRadius: "6px", border: "1px solid rgba(255,255,255,0.05)" }}>
            <strong style={{ color: "#00f3ff", display: "block", marginBottom: "0.3rem" }}>4️⃣ إكرامية جيتو (Jito Tip) وحماية الصفقات</strong>
            دي إكرامية صغيرة بتندفع لمعدني جيتو عشان يمرروا الصفقة في حزمة خاصة (Private MEV Bundle) لمنع السرقة أو الفرونت-رن (Front-running) وضمان دخول المعاملة بأمان وسرعة فائقة.
          </div>
        </div>
      )}
    </div>
  );
}
