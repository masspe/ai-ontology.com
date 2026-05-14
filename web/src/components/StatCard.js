import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
function fmt(n) {
    if (typeof n === "string")
        return n;
    if (n >= 1_000_000)
        return (n / 1_000_000).toFixed(1) + "M";
    if (n >= 10_000)
        return (n / 1_000).toFixed(1) + "k";
    return String(n);
}
export default function StatCard({ label, value, deltaPct, hint }) {
    const cls = deltaPct == null ? "flat" : deltaPct > 0.5 ? "up" : deltaPct < -0.5 ? "down" : "flat";
    const arrow = cls === "up" ? "▲" : cls === "down" ? "▼" : "•";
    return (_jsxs("div", { className: "card stat-card", children: [_jsx("span", { className: "label", children: label }), _jsx("span", { className: "value", children: fmt(value) }), deltaPct != null && (_jsxs("span", { className: `delta ${cls}`, children: [arrow, " ", Math.abs(deltaPct).toFixed(1), "% ", hint ?? "vs first sample"] })), deltaPct == null && hint && _jsx("span", { className: "delta flat", children: hint })] }));
}
