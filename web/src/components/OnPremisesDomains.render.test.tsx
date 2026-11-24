// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// The on-premises card of the data-model page: the domains come from the
// types, a tick and the provider are saved at once, and a kept domain
// without a provider says its questions are refused.

import { beforeEach, describe, expect, it, vi } from "vitest";
import OnPremisesDomains, { domainsOf } from "./OnPremisesDomains";
import { fireEvent, flushPromises, renderPage, screen, waitFor } from "../test/render";
import type { Ontology } from "../api";

vi.mock("../api", async () => {
  const actual = await vi.importActual<typeof import("../api")>("../api");
  return { ...actual, getSettings: vi.fn(), patchSettings: vi.fn() };
});
import * as api from "../api";
const m = api as unknown as Record<string, ReturnType<typeof vi.fn>>;

const ontology = {
  concept_types: { Person: { ns: "hr" }, Employee: { parent: "Person" }, Order: {}, Loop: { parent: "Loop" }, Orphan: { parent: "Gone" } },
  relation_types: {},
} as unknown as Ontology;

const settings = (domains: string[], provider = "") => ({ llm: { on_premises_domains: domains, on_premises_provider: provider } });

beforeEach(() => {
  m.getSettings.mockResolvedValue(settings(["legacy"]));
  m.patchSettings.mockImplementation((p: { llm: { on_premises_domains?: string[]; on_premises_provider?: string } }) =>
    Promise.resolve(settings(p.llm.on_premises_domains ?? ["legacy", "hr"], p.llm.on_premises_provider ?? "")),
  );
});

describe("On-premises domains", () => {
  it("gives a type its own domain, else its nearest ancestor's, else the default one", () => {
    expect(domainsOf(ontology)).toEqual({ hr: 2, default: 3 });
  });

  it("ticks a domain, names the provider and warns while there is none", async () => {
    renderPage(<OnPremisesDomains ontology={ontology} />);
    const hr = await screen.findByRole("checkbox", { name: /^hr/ });
    expect(hr).not.toBeChecked();
    // A kept domain no type has any more stays listed, to be released.
    expect(screen.getByRole("checkbox", { name: /^legacy.*0 type/ })).toBeChecked();
    expect(screen.getByRole("status")).toHaveTextContent("Aucun fournisseur sur site");

    fireEvent.click(hr);
    await waitFor(() => expect(m.patchSettings).toHaveBeenCalledWith({ llm: { on_premises_domains: ["legacy", "hr"] } }));
    await waitFor(() => expect(screen.getByRole("checkbox", { name: /^hr/ })).toBeChecked());

    fireEvent.change(screen.getByLabelText("Fournisseur sur site"), { target: { value: "infomaniak" } });
    await waitFor(() => expect(m.patchSettings).toHaveBeenCalledWith({ llm: { on_premises_provider: "infomaniak" } }));
    await waitFor(() => expect(screen.queryByRole("status")).toBeNull());

    fireEvent.click(screen.getByRole("checkbox", { name: /^legacy/ }));
    await waitFor(() => expect(m.patchSettings).toHaveBeenLastCalledWith({ llm: { on_premises_domains: ["hr"] } }));
    await waitFor(() => expect(screen.queryByRole("checkbox", { name: /^legacy/ })).toBeNull());
  });

  it("shows a refused save and keeps what was ticked", async () => {
    m.patchSettings.mockRejectedValueOnce(new Error("settings are for administrators")).mockRejectedValueOnce("offline");
    renderPage(<OnPremisesDomains ontology={ontology} />);
    fireEvent.click(await screen.findByRole("checkbox", { name: /^hr/ }));
    expect(await screen.findByRole("alert")).toHaveTextContent("settings are for administrators");
    expect(screen.getByRole("checkbox", { name: /^hr/ })).not.toBeChecked();
    fireEvent.click(screen.getByRole("checkbox", { name: /^hr/ }));
    await waitFor(() => expect(screen.getByRole("alert")).toHaveTextContent("offline"));
  });

  it("is absent without a model, without settings, and without a domain", async () => {
    const absent = async (ui: React.ReactElement) => {
      const r = renderPage(ui);
      await flushPromises();
      expect(screen.queryByText("Domaines sur site")).toBeNull();
      r.unmount();
    };
    await absent(<OnPremisesDomains ontology={null} />);
    m.getSettings.mockRejectedValueOnce(new Error("401"));
    await absent(<OnPremisesDomains ontology={ontology} />);
    // Settings from a server that predates the boundary: no field at all.
    m.getSettings.mockResolvedValueOnce({ llm: {} });
    await absent(<OnPremisesDomains ontology={{ concept_types: {}, relation_types: {} } as unknown as Ontology} />);
    expect(m.getSettings).toHaveBeenCalledTimes(3);
  });
});
