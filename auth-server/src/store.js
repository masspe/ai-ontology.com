// SPDX-License-Identifier: AGPL-3.0-or-later
// Minimal JSON-file-backed user store for development.
// SWAP THIS for a real DB (Postgres, SQLite, etc.) in production.
import { promises as fs } from "node:fs";
import path from "node:path";
import { randomUUID } from "node:crypto";
import { config } from "./config.js";

let cache = null;
let writing = Promise.resolve();

async function load() {
  if (cache) return cache;
  try {
    const raw = await fs.readFile(config.usersFile, "utf8");
    cache = JSON.parse(raw);
  } catch (e) {
    if (e.code !== "ENOENT") throw e;
    cache = { users: [] };
  }
  return cache;
}

async function persist() {
  writing = writing.then(async () => {
    await fs.mkdir(path.dirname(config.usersFile), { recursive: true });
    await fs.writeFile(config.usersFile, JSON.stringify(cache, null, 2), "utf8");
  });
  return writing;
}

function publicUser(u) {
  if (!u) return null;
  const { passwordHash, providers, ...rest } = u;
  return { ...rest, providers: providers?.map((p) => ({ provider: p.provider, sub: p.sub })) ?? [] };
}

export const users = {
  async findByEmail(email) {
    const db = await load();
    const lc = String(email || "").toLowerCase();
    return db.users.find((u) => u.email === lc) || null;
  },
  async findById(id) {
    const db = await load();
    return db.users.find((u) => u.id === id) || null;
  },
  async findByProvider(provider, sub) {
    const db = await load();
    return db.users.find((u) => u.providers?.some((p) => p.provider === provider && p.sub === sub)) || null;
  },
  async create({ email, name, passwordHash = null, provider = null, sub = null, picture = null }) {
    const db = await load();
    const lc = String(email).toLowerCase();
    if (db.users.some((u) => u.email === lc)) {
      const err = new Error("EMAIL_TAKEN");
      err.code = "EMAIL_TAKEN";
      throw err;
    }
    const user = {
      id: randomUUID(),
      email: lc,
      name: name || lc.split("@")[0],
      picture,
      passwordHash,
      providers: provider ? [{ provider, sub }] : [],
      createdAt: new Date().toISOString(),
    };
    db.users.push(user);
    await persist();
    return user;
  },
  async linkProvider(userId, provider, sub) {
    const db = await load();
    const u = db.users.find((x) => x.id === userId);
    if (!u) return null;
    u.providers = u.providers || [];
    if (!u.providers.some((p) => p.provider === provider && p.sub === sub)) {
      u.providers.push({ provider, sub });
      await persist();
    }
    return u;
  },
  publicUser,
};
