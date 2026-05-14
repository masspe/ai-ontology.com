// SPDX-License-Identifier: AGPL-3.0-or-later
import express from "express";
import cors from "cors";
import helmet from "helmet";
import cookieParser from "cookie-parser";
import { pathToFileURL } from "node:url";
import { config } from "./config.js";
import authRoutes from "./routes/auth.js";
import oauthRoutes from "./routes/oauth.js";

export function buildApp() {
  const app = express();
  app.disable("x-powered-by");
  app.use(helmet());
  app.use(cookieParser());
  app.use(express.json({ limit: "100kb" }));
  app.use(cors({
    origin(origin, cb) {
      if (!origin) return cb(null, true); // server-to-server / curl
      if (config.webOrigins.includes(origin)) return cb(null, true);
      return cb(new Error(`Origin ${origin} not allowed`));
    },
    credentials: true,
  }));

  app.get("/healthz", (_req, res) => res.json({ ok: true }));
  app.use("/auth", authRoutes);
  app.use("/auth/oauth", oauthRoutes);

  // Generic error handler
  app.use((err, _req, res, _next) => {
    console.error(err);
    res.status(err.status || 500).json({ error: err.message || "Internal error" });
  });
  return app;
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) {
  const app = buildApp();
  app.listen(config.port, () => {
    console.log(`[auth-server] listening on http://localhost:${config.port}`);
  });
}
