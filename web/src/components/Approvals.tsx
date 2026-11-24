// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// "Écritures à approuver": what actions asked to write back to a data
// source. The statement shown is the one that runs, and another
// administrator than the one who asked approves it. Hidden while nothing
// was asked, and from who may not read the list.

import { useEffect, useState } from "react";
import Card from "./Card";
// @ts-expect-error JSX module
import { useConfirm } from "./ConfirmDialog.jsx";
import { decideApproval, listApprovals, type Approval } from "../api";
import { t } from "../lib/i18n";

const BADGE: Record<Approval["status"], string> = {
  pending: "badge-warn",
  running: "badge-accent",
  done: "badge-success",
  failed: "badge-danger",
  rejected: "badge-danger",
};

function statusLabel(s: Approval["status"]): string {
  switch (s) {
    case "pending":
      return t("en attente");
    case "running":
      return t("en cours");
    case "done":
      return t("exécutée");
    case "failed":
      return t("échouée");
    default:
      return t("refusée");
  }
}

/** `reload` changes when the page asked for a new write. */
export default function Approvals({ reload }: { reload: number }) {
  const confirm = useConfirm();
  const [rows, setRows] = useState<Approval[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [decided, setDecided] = useState(0);

  useEffect(() => {
    listApprovals().then(
      (r) => setRows(r.approvals),
      () => setRows([]),
    );
  }, [reload, decided]);

  if (rows.length === 0) return null;

  const decide = async (a: Approval, decision: "approve" | "reject") => {
    const approve = decision === "approve";
    const ok = await confirm({
      title: approve ? t("Approuver l'écriture") : t("Refuser l'écriture"),
      message: approve
        ? t("Cette instruction sera exécutée sur {source} : {sql}", { source: a.source_name, sql: a.sql })
        : t("Refuser cette demande ? Rien ne sera écrit."),
      confirmLabel: approve ? t("Approuver") : t("Refuser"),
      danger: true,
    });
    if (!ok) return;
    try {
      await decideApproval(a.id, decision);
      setError(null);
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    }
    setDecided((n) => n + 1);
  };

  return (
    <Card
      title={t("Écritures à approuver")}
      subtitle={t("Une action demandée n'écrit dans sa source qu'une fois approuvée par un autre administrateur.")}
      style={{ marginBottom: 16 }}
    >
      {error && <div className="error-banner" role="alert">{error}</div>}
      <table className="table">
        <thead>
          <tr>
            <th>{t("Action")}</th>
            <th>{t("Instruction")}</th>
            <th>{t("Demandée par")}</th>
            <th>{t("Statut")}</th>
            <th>{t("Décision")}</th>
          </tr>
        </thead>
        <tbody>
          {rows.slice(0, 10).map((a) => (
            <tr key={a.id}>
              <td>
                <strong>{a.action_name}</strong>
                <div className="muted">{a.source_name}</div>
              </td>
              <td>
                <code>{a.sql}</code>
                <div className="muted">{t("{n} ligne(s) au plus", { n: a.max_rows })}</div>
              </td>
              <td>
                {a.requested_by}
                <div className="muted">{new Date(a.requested_at).toLocaleString()}</div>
              </td>
              <td>
                <span className={`badge ${BADGE[a.status]}`}>{statusLabel(a.status)}</span>
                {a.rows_affected != null && <div className="muted">{t("{n} ligne(s) modifiée(s)", { n: a.rows_affected })}</div>}
                {a.error && <div className="muted">{a.error}</div>}
              </td>
              <td>
                {a.status === "pending" ? (
                  <>
                    <button className="btn-ghost" onClick={() => void decide(a, "approve")}>{t("Approuver")}</button>
                    <button className="btn-ghost" onClick={() => void decide(a, "reject")}>{t("Refuser")}</button>
                  </>
                ) : (
                  a.decided_by
                )}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </Card>
  );
}
