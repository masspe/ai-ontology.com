// SPDX-License-Identifier: AGPL-3.0-or-later
import "dotenv/config";

function required(name) {
  const v = process.env[name];
  if (!v || !v.trim()) {
    if (process.env.NODE_ENV === "production") {
      throw new Error(`Missing required env var: ${name}`);
    }
    console.warn(`[config] WARN: ${name} is empty (ok for dev)`);
  }
  return v || "";
}

export const config = {
  port: parseInt(process.env.PORT || "4000", 10),
  nodeEnv: process.env.NODE_ENV || "development",
  webOrigins: (process.env.WEB_ORIGIN || "http://localhost:5173")
    .split(",").map((s) => s.trim()).filter(Boolean),
  frontendOAuthCallback: process.env.OAUTH_FRONTEND_CALLBACK
    || "http://localhost:5173/oauth/callback",

  jwtSecret: required("JWT_SECRET"),
  jwtExpiresIn: process.env.JWT_EXPIRES_IN || "7d",
  oauthStateSecret: required("OAUTH_STATE_SECRET") || "dev-state-secret",

  google: {
    clientId: process.env.GOOGLE_CLIENT_ID || "",
    clientSecret: process.env.GOOGLE_CLIENT_SECRET || "",
    redirectUri: process.env.GOOGLE_REDIRECT_URI
      || "http://localhost:4000/auth/oauth/google/callback",
  },
  microsoft: {
    clientId: process.env.MICROSOFT_CLIENT_ID || "",
    clientSecret: process.env.MICROSOFT_CLIENT_SECRET || "",
    tenant: process.env.MICROSOFT_TENANT || "common",
    redirectUri: process.env.MICROSOFT_REDIRECT_URI
      || "http://localhost:4000/auth/oauth/microsoft/callback",
  },

  usersFile: process.env.USERS_FILE || "./data/users.json",
};

export function isProd() { return config.nodeEnv === "production"; }
