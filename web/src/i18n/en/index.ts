// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// English dictionary, French sentence -> English sentence, one file per area
// so that several people can extend it without touching the same file.
// `src/lib/i18n.test.ts` fails when a `t("…")` key of the sources has no
// entry here.

import { layout } from "./layout";
import { components } from "./components";
import { pagesA } from "./pages-a";
import { pagesB } from "./pages-b";

export const en: Record<string, string> = { ...layout, ...components, ...pagesA, ...pagesB };
