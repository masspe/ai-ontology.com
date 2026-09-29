// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// The menu, in the order of the work (ROADMAP §3.9 lot A): prepare, then
// explore, then automate. One source for the sidebar and the guide: each
// entry carries its `tour` anchor and the two sentences the guide says.

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
  /** `data-tour` anchor and guide step id. */
  tour: string;
  /** What the guide says about this part, two sentences at most. */
  guide: string;
}

export interface NavGroup {
  title: string;
  items: NavItem[];
}

export const NAV_GROUPS: NavGroup[] = [
  {
    title: "Accueil",
    items: [
      {
        to: "/",
        label: "Tableau de bord",
        icon: NavIcon.home,
        end: true,
        tour: "dashboard",
        guide: "La vue d'ensemble : taille du graphe, fichiers importés, raccourcis. Au premier lancement, il vous guide en trois étapes.",
      },
    ],
  },
  {
    title: "Préparer",
    items: [
      {
        to: "/builder",
        label: "Modèle de données",
        icon: NavIcon.layers,
        tour: "builder",
        guide: "Les types de fiches (Personne, Contrat…), leurs propriétés et les relations permises. Il peut être généré à partir d'une description en langage courant.",
      },
      {
        to: "/files",
        label: "Fichiers",
        icon: NavIcon.folder,
        tour: "files",
        guide: "Déposez ici vos Word, Excel, CSV, PDF ou textes. Chaque fichier reste listé avec l'état de son import.",
      },
      {
        to: "/ingest",
        label: "Importer des documents",
        icon: NavIcon.upload,
        tour: "ingest",
        guide: "Un modèle de langage lit un fichier et propose des fiches et des relations. Vous relisez, corrigez, puis validez ce qui entre dans le graphe.",
      },
    ],
  },
  {
    title: "Explorer",
    items: [
      {
        to: "/graph",
        label: "Graphe",
        icon: NavIcon.graph,
        tour: "graph",
        guide: "Les liens autour d'une fiche, à un ou deux pas, filtrés par type. Le bon endroit pour comprendre comment les choses se tiennent.",
      },
      {
        to: "/concepts",
        label: "Fiches",
        icon: NavIcon.list,
        tour: "concepts",
        guide: "Toutes les fiches du graphe : création, modification, relations, suppression groupée.",
      },
      {
        to: "/queries",
        label: "Questions",
        icon: NavIcon.search,
        tour: "queries",
        guide: "Posez une question en langage courant ; la réponse cite les fiches d'où elle vient. Les questions utiles se gardent pour être rejouées.",
      },
    ],
  },
  {
    title: "Automatiser",
    items: [
      {
        to: "/rules",
        label: "Règles",
        icon: NavIcon.shield,
        tour: "rules",
        guide: "Des règles de cohérence sur le graphe, qui signalent les exceptions plutôt que de les laisser passer.",
      },
      {
        to: "/actions",
        label: "Actions",
        icon: NavIcon.bolt,
        tour: "actions",
        guide: "Des automatisations déclenchées par le contenu du graphe.",
      },
    ],
  },
  {
    title: "Réglages",
    items: [
      {
        to: "/settings",
        label: "Paramètres",
        icon: NavIcon.settings,
        tour: "settings",
        guide: "Le fournisseur d'IA et ses clés, les diagnostics, la mémoire. Rien ici n'est nécessaire pour commencer.",
      },
    ],
  },
];

export const NAV_ITEMS: NavItem[] = NAV_GROUPS.flatMap((g) => g.items);
