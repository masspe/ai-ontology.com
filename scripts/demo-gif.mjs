// Records docs/demo.gif: drives the finance walkthrough in the web UI
// (empty home → one-click example → a question → the graph, filtered, one
// sheet selected) and saves the video. Replay it after any UI change.
//
// Needs the three dev servers up (`cd web && npm run dev`) on an EMPTY
// store, an account demo@example.com / demo-pass-123 (sign up once), and
// an LLM provider in <data>/settings.json (the echo model makes a dull
// answer). Then, from scripts/:
//
//   npm i --no-save playwright ffmpeg-static && npx playwright install chromium
//   node demo-gif.mjs            # writes out/<id>.webm (--shots: PNGs only, --fast: no pauses)
//   F=$(node -p "require('ffmpeg-static')"); V=$(ls out/*.webm)
//   # The canvas goes blank for ~2 frames when a sheet is selected (and the
//   # page flashes white on navigation): invisible live, a blink in a GIF.
//   # Find them (canvas band brighter than its neighbours) and drop them:
//   $F -i $V -vf "select='not(eq(n\,A)+eq(n\,B))',setpts=N/25/TB" -r 25 -c:v libx264 -pix_fmt yuv420p -crf 26 -movflags +faststart -y ../docs/demo.mp4
//   $F -i ../docs/demo.mp4 -vf "fps=8,scale=880:-1:flags=lanczos,palettegen=max_colors=256:stats_mode=diff" -y out/pal.png
//   $F -i ../docs/demo.mp4 -i out/pal.png -lavfi "fps=8,scale=880:-1:flags=lanczos[x];[x][1:v]paletteuse=dither=none:diff_mode=rectangle" -y ../docs/demo.gif
import { chromium } from "playwright";
import fs from "node:fs";

const SHOTS = process.argv.includes("--shots");
const FAST = process.argv.includes("--fast");
// Override when the dev stack runs on other ports (e.g. next to the user's own).
const WEB = process.env.DEMO_WEB ?? "http://localhost:5173";
const AUTH = process.env.DEMO_AUTH ?? "http://localhost:4000";
const OUT = new URL("./out/", import.meta.url).pathname.replace(/^\/([A-Z]:)/, "$1");
fs.mkdirSync(OUT, { recursive: true });

const token = (await (await fetch(`${AUTH}/auth/login`, {
  method: "POST", headers: { "content-type": "application/json" },
  body: JSON.stringify({ email: "demo@example.com", password: "demo-pass-123" }),
})).json());

const browser = await chromium.launch();
const ctx = await browser.newContext({
  viewport: { width: 1280, height: 800 },
  deviceScaleFactor: 1,
  recordVideo: SHOTS ? undefined : { dir: OUT, size: { width: 1280, height: 800 } },
});
await ctx.addInitScript((t) => {
  localStorage.setItem("msBE.token", t.token);
  localStorage.setItem("msBE.user", JSON.stringify(t.user));
  localStorage.setItem("tour.v1", JSON.stringify({ done: true, at: Date.now() }));
}, token);

const page = await ctx.newPage();
let n = 0;
const shot = async (name) => { if (SHOTS) await page.screenshot({ path: `${OUT}${String(++n).padStart(2, "0")}-${name}.png` }); };
const pause = (ms) => page.waitForTimeout(FAST ? 300 : ms);
// Move the mouse to an element before clicking so the cursor path reads on video.
const click = async (loc) => { await loc.hover(); await pause(400); await loc.click(); };

// 1. Empty home: the onboarding steps.
await page.goto(`${WEB}/`);
await page.getByText("Décrivez vos données").waitFor();
await pause(1500);
await shot("home-empty");

// 2. One click loads the finance example; the stats grow live.
await click(page.getByRole("button", { name: /exemple finance/ }));
await page.getByText(/\d+ fiches et \d+ liens/).waitFor({ timeout: 60000 });
await pause(2000);
await shot("home-loaded");

// 3. Ask a question from the home search bar.
const q = page.getByLabel("Question");
await click(q);
await q.pressSequentially("Quelles factures ont été émises à Initech, et pour quels contrats ?", { delay: 45 });
await pause(500);
await page.keyboard.press("Enter");
await page.getByPlaceholder("Posez votre question…").waitFor();
await pause(800);
await click(page.getByRole("button", { name: "Poser la question" }));
await page.getByText(/Réponse fondée sur/).waitFor({ timeout: 90000 });
await page.getByRole("button", { name: "Poser la question" }).waitFor({ timeout: 90000 }); // streaming done
await pause(3500);
await shot("answer");

// 4. The graph with its filters: keep the business sheets, drop the line items.
await page.goto(`${WEB}/graph`);
await page.locator(".react-flow__node").first().waitFor({ timeout: 30000 });
await pause(1200);
const scrollTo = (sel) => page.evaluate((q) => document.querySelector(q)?.scrollIntoView({ block: "start", behavior: "smooth" }), sel);
await scrollTo(".gv-canvas-card");
await pause(600);
await click(page.getByRole("button", { name: "Ajuster la vue" }));
await pause(2500);
await shot("graph");
await click(page.getByRole("button", { name: /Types de fiche/ }));
await pause(600);
const types = page.getByRole("dialog", { name: "Types de fiche" });
// Each tick reloads and re-fits the subgraph: let every state settle so the
// viewer sees three steps, not a flicker.
for (const t of ["Company", "Person", "Contract"]) { await click(types.getByLabel(t, { exact: true })); await pause(1500); }
await page.keyboard.press("Escape");
await page.waitForTimeout(2000); // let the last subgraph request land before fitting
await click(page.getByRole("button", { name: "Ajuster la vue" }));
await pause(2000);
await shot("graph-filtered");

// 5. Click a sheet: the inspector shows its links.
const node = page.locator(".react-flow__node", { hasText: "C-2025-002" }).first();
await node.waitFor();
await click(node);
await page.locator(".gv-inspector-name", { hasText: "C-2025-002" }).waitFor();
await pause(2500);
// Park the cursor off the canvas first: during the scroll it would sweep over nodes and light them up.
await page.mouse.move(1265, 400);
await pause(300);
await page.evaluate(() => document.querySelector(".gv-inspector-card")?.scrollIntoView({ block: "start" })); // instant: a smooth scroll makes ReactFlow redraw mid-way
await pause(3000);
await shot("graph-node");

await ctx.close();
await browser.close();
console.log("done", OUT);
