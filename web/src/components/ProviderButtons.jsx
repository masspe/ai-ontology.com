// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// One button per sign-in provider the server has configured (Settings →
// Sign-in): nothing is shown while none is. `verb` turns the provider's
// label into the button text ("Continue with Microsoft").
import { useEffect, useState } from "react";
import { msBE } from "../lib/msBE";
import { t } from "../lib/i18n";

const STYLES = {
  google: ["#fff", "#111", "#ddd"],
  microsoft: ["#2f2f2f", "#fff", "#2f2f2f"],
};

export default function ProviderButtons({ next, verb }) {
  const [providers, setProviders] = useState([]);
  useEffect(() => {
    let on = true;
    Promise.resolve(msBE.auth.providers()).then((list) => {
      if (on) setProviders(Array.isArray(list) ? list : []);
    });
    return () => { on = false; };
  }, []);
  if (providers.length === 0) return null;
  return (
    <>
      <div style={divider}><span>{t("ou")}</span></div>
      {providers.map((p) => {
        const [bg, fg, bd] = STYLES[p.id] || ["var(--panel-2, #f4f4f5)", "#111", "#d4d4d8"];
        return (
          <a key={p.id} href={msBE.auth.providerLoginUrl(p.id, next)} style={oauthBtn(bg, fg, bd)}>
            {verb(p.label)}
          </a>
        );
      })}
    </>
  );
}

const divider = { textAlign: "center", color: "#6b7280", fontSize: 12, margin: "8px 0" };
const oauthBtn = (bg, fg, bd) => ({ display: "block", textAlign: "center", padding: "10px 14px", background: bg, color: fg, border: `1px solid ${bd}`, borderRadius: 8, textDecoration: "none", fontWeight: 600 });
