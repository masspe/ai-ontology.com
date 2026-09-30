// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// The menu (ROADMAP §3.9): one source for the top navigation and the
// guide: each entry carries its `tour` anchor and the two sentences the
// guide says.

import type { ReactElement } from "react";

const svg = (paths: ReactElement) => (
  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden>
    {paths}
  </svg>
);

export const NavIcon = {
  home: svg(<><path d="m3 11 9-8 9 8" /><path d="M5 10v10h14V10" /></>),
  layers: svg(<><path d="M12 2 2 7l10 5 10-5-10-5z" /><path d="m2 17 10 5 10-5" /><path d="m2 12 10 5 10-5" /></>),
  folder: svg(<path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V7z" />),
  upload: svg(<><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" /><path d="m17 8-5-5-5 5" /><path d="M12 3v12" /></>),
  graph: svg(<><circle cx="5" cy="12" r="2.5" /><circle cx="19" cy="5" r="2.5" /><circle cx="19" cy="19" r="2.5" /><path d="m7 11 10-5" /><path d="m7 13 10 5" /></>),
  list: svg(<><path d="M8 6h13" /><path d="M8 12h13" /><path d="M8 18h13" /><path d="M3 6h.01" /><path d="M3 12h.01" /><path d="M3 18h.01" /></>),
  search: svg(<><circle cx="11" cy="11" r="7" /><path d="m21 21-4.3-4.3" /></>),
  shield: svg(<><path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z" /><path d="m9 12 2 2 4-4" /></>),
  bolt: svg(<path d="M13 2 3 14h8l-1 8 10-12h-8l1-8z" />),
  settings: svg(<><circle cx="12" cy="12" r="3" /><path d="M19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.7 1.7 0 0 0 1.5-1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1z" /></>),
};

export interface NavItem {
  to: string;
  label: string;
  icon: ReactElement;
  /** Exact match (the index route). */
  end?: boolean;
  /** `data-tour` anchor the guide spotlights: the entry itself, or the
   * menu it lives in. */
  tour: string;
  /** What the guide says about this part, two sentences at most. */
  guide: string;
}

export interface NavGroup {
  /** `data-tour` anchor of the menu; also the id of the group. */
  key: string;
  title: string;
  items: NavItem[];
}

// Four entries (decided with the owner on 2026-09-30): Accueil and
// Réglages are one page each, Importer and Explorer open a menu.
export const NAV_GROUPS: NavGroup[] = [
  {
    key: "accueil",
    title: "Accueil",
    items: [
      {
        to: "/",
        label: "Accueil",
        icon: NavIcon.home,
        end: true,
        tour: "accueil",
        guide: "Une question à poser, ce qui attend une action, ce qui a été importé cette semaine, les dernières questions. Au premier lancement, il vous guide en trois étapes.",
      },
    ],
  },
  {
    key: "importer",
    title: "Importer",
    items: [
      {
        to: "/files",
        label: "Fichiers",
        icon: NavIcon.folder,
        tour: "importer",
        guide: "Déposez ici vos Word, Excel, CSV, PDF ou textes. Un document part en relecture, un fichier structuré se charge directement ; chacun reste listé avec l'état de son import.",
      },
      {
        to: "/ingest",
        label: "Relire un document",
        icon: NavIcon.upload,
        tour: "importer",
        guide: "Un modèle de langage lit un document et propose des fiches et des liens. Vous relisez, corrigez, puis ajoutez ce qui entre dans vos données.",
      },
      {
        to: "/builder",
        label: "Modèle de données",
        icon: NavIcon.layers,
        tour: "importer",
        guide: "Les types de fiches (Personne, Contrat…), leurs propriétés et les liens permis. Il peut être généré à partir d'une description en langage courant.",
      },
    ],
  },
  {
    key: "explorer",
    title: "Explorer",
    items: [
      {
        to: "/concepts",
        label: "Fiches",
        icon: NavIcon.list,
        tour: "explorer",
        guide: "Toutes les fiches. Chacune s'ouvre sur sa page : liens, informations, documents d'origine, règles, correction sur place.",
      },
      {
        to: "/graph",
        label: "Graphe",
        icon: NavIcon.graph,
        tour: "explorer",
        guide: "Les liens autour d'une fiche, à un ou deux pas, filtrés par type. Le bon endroit pour comprendre comment les choses se tiennent.",
      },
      {
        to: "/queries",
        label: "Questions",
        icon: NavIcon.search,
        tour: "explorer",
        guide: "Posez une question en langage courant ; la réponse cite les fiches d'où elle vient. Les questions utiles se gardent, se rejouent et s'épinglent à l'accueil.",
      },
      {
        to: "/rules",
        label: "Règles",
        icon: NavIcon.shield,
        tour: "explorer",
        guide: "Des règles de cohérence sur vos données, qui signalent les exceptions plutôt que de les laisser passer.",
      },
      {
        to: "/actions",
        label: "Actions",
        icon: NavIcon.bolt,
        tour: "explorer",
        guide: "Des automatisations déclenchées par le contenu de vos données.",
      },
    ],
  },
  {
    key: "reglages",
    title: "Réglages",
    items: [
      {
        to: "/settings",
        label: "Réglages",
        icon: NavIcon.settings,
        tour: "reglages",
        guide: "Le fournisseur d'IA et ses clés, les diagnostics, la mémoire. Rien ici n'est nécessaire pour commencer.",
      },
    ],
  },
];

export const NAV_ITEMS: NavItem[] = NAV_GROUPS.flatMap((g) => g.items);
