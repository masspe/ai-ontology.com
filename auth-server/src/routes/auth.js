// SPDX-License-Identifier: AGPL-3.0-or-later
import { Router } from "express";
import bcrypt from "bcryptjs";
import rateLimit from "express-rate-limit";
import { users } from "../store.js";
import { signJwt } from "../jwt.js";
import { requireAuth } from "../middleware.js";

const router = Router();

const EMAIL_RE = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;

const writeLimiter = rateLimit({
  windowMs: 15 * 60 * 1000,
  limit: 20,
  standardHeaders: "draft-7",
  legacyHeaders: false,
});

function strongEnough(pw) {
  if (typeof pw !== "string" || pw.length < 8) return false;
  let cls = 0;
  if (/[A-Z]/.test(pw)) cls++;
  if (/[a-z]/.test(pw)) cls++;
  if (/\d/.test(pw)) cls++;
  if (/[^A-Za-z0-9]/.test(pw)) cls++;
  return cls >= 3;
}

router.post("/signup", writeLimiter, async (req, res) => {
  const { email, password, name } = req.body || {};
  if (!EMAIL_RE.test(String(email || ""))) {
    return res.status(400).json({ error: "Invalid email" });
  }
  if (!strongEnough(password)) {
    return res.status(400).json({ error: "Weak password (min 8 chars, mixed classes)" });
  }
  if (typeof name !== "string" || !name.trim()) {
    return res.status(400).json({ error: "Name required" });
  }
  try {
    const passwordHash = await bcrypt.hash(password, 12);
    const user = await users.create({ email, name: name.trim(), passwordHash });
    const token = signJwt(user);
    return res.status(201).json({ token, user: users.publicUser(user) });
  } catch (e) {
    if (e.code === "EMAIL_TAKEN") {
      return res.status(409).json({ error: "Email already registered" });
    }
    console.error(e);
    return res.status(500).json({ error: "Internal error" });
  }
});

router.post("/login", writeLimiter, async (req, res) => {
  const { email, password } = req.body || {};
  if (!EMAIL_RE.test(String(email || "")) || typeof password !== "string") {
    return res.status(400).json({ error: "Invalid credentials" });
  }
  const user = await users.findByEmail(email);
  // Constant-ish time: still hash-compare a dummy when user not found.
  const hash = user?.passwordHash || "$2a$12$0000000000000000000000.0000000000000000000000000000";
  const ok = await bcrypt.compare(password, hash);
  if (!user || !user.passwordHash || !ok) {
    return res.status(401).json({ error: "Invalid credentials" });
  }
  const token = signJwt(user);
  return res.json({ token, user: users.publicUser(user) });
});

router.get("/me", requireAuth, (req, res) => {
  res.json({ user: req.user });
});

router.post("/logout", (_req, res) => {
  // Stateless JWT: client discards the token. (Hook for token denylist here.)
  res.json({ ok: true });
});

export default router;
