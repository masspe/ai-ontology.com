import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
/** Plain inline SVG sparkline — no external deps. */
export default function Sparkline({ values, stroke = "var(--accent)" }) {
    if (values.length === 0) {
        return _jsx("svg", { className: "sparkline", viewBox: "0 0 100 36", preserveAspectRatio: "none" });
    }
    const min = Math.min(...values);
    const max = Math.max(...values);
    const span = max - min || 1;
    const stepX = values.length > 1 ? 100 / (values.length - 1) : 0;
    const points = values
        .map((v, i) => {
        const x = i * stepX;
        const y = 32 - ((v - min) / span) * 28 + 2;
        return `${x.toFixed(2)},${y.toFixed(2)}`;
    })
        .join(" ");
    const area = `0,36 ${points} 100,36`;
    return (_jsxs("svg", { className: "sparkline", viewBox: "0 0 100 36", preserveAspectRatio: "none", children: [_jsx("polygon", { points: area, fill: "var(--accent-soft)", opacity: 0.6 }), _jsx("polyline", { points: points, fill: "none", stroke: stroke, strokeWidth: 1.6 })] }));
}
