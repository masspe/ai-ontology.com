// SPDX-License-Identifier: AGPL-3.0-or-later
// Run with: npm test
import test from "node:test";
import assert from "node:assert/strict";
import { promises as fs } from "node:fs";
import path from "node:path";
import os from "node:os";

// Isolate the user store + secrets BEFORE importing the app.
const tmpDir = await fs.mkdtemp(path.join(os.tmpdir(), "auth-srv-"));
process.env.USERS_FILE = path.join(tmpDir, "users.json");
process.env.JWT_SECRET = "test-jwt-secret-test-jwt-secret-test-jwt-secret";
process.env.OAUTH_STATE_SECRET = "test-state-secret-test-state-secret";
process.env.NODE_ENV = "test";

const { buildApp } = await import("../src/index.js");
const { default: supertest } = await import("supertest");

const app = buildApp();
const agent = supertest(app);

test("healthz", async () => {
  const r = await agent.get("/healthz");
  assert.equal(r.status, 200);
  assert.equal(r.body.ok, true);
});

test("signup rejects weak password", async () => {
  const r = await agent.post("/auth/signup").send({ email: "a@b.co", password: "123", name: "A" });
  assert.equal(r.status, 400);
});

test("signup -> me -> login -> logout", async () => {
  const creds = { email: "alice@example.com", password: "Str0ngPass!", name: "Alice" };
  const s = await agent.post("/auth/signup").send(creds);
  assert.equal(s.status, 201);
  assert.ok(s.body.token, "token returned");
  assert.equal(s.body.user.email, creds.email);
  const token = s.body.token;

  const me = await agent.get("/auth/me").set("authorization", `Bearer ${token}`);
  assert.equal(me.status, 200);
  assert.equal(me.body.user.email, creds.email);

  const dup = await agent.post("/auth/signup").send(creds);
  assert.equal(dup.status, 409);

  const lo = await agent.post("/auth/login").send({ email: creds.email, password: creds.password });
  assert.equal(lo.status, 200);
  assert.ok(lo.body.token);

  const bad = await agent.post("/auth/login").send({ email: creds.email, password: "wrong" });
  assert.equal(bad.status, 401);

  const out = await agent.post("/auth/logout");
  assert.equal(out.status, 200);
});

test("me requires bearer", async () => {
  const r = await agent.get("/auth/me");
  assert.equal(r.status, 401);
});

test("me rejects tampered token", async () => {
  const r = await agent.get("/auth/me").set("authorization", "Bearer not.a.jwt");
  assert.equal(r.status, 401);
});

test("oauth start returns 503 when not configured", async () => {
  const r = await agent.get("/auth/oauth/google/start").redirects(0);
  assert.equal(r.status, 503);
});
