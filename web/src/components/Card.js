import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
export default function Card({ title, subtitle, actions, children, className, style }) {
    return (_jsxs("section", { className: `card${className ? " " + className : ""}`, style: style, children: [(title || actions) && (_jsxs("header", { className: "card-title", children: [_jsx("span", { children: title }), actions && _jsx("span", { children: actions })] })), subtitle && _jsx("div", { className: "card-subtitle", children: subtitle }), children] }));
}
