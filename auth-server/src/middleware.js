// SPDX-License-Identifier: AGPL-3.0-or-later
import { verifyJwt } from "./jwt.js";
import { users } from "./store.js";

/** Express middleware: requires a valid Bearer JWT; attaches req.user. */
export async function requireAuth(req, res, next) {
  try {
    const h = req.headers.authorization || "";
    const m = /^Bearer\s+(.+)$/i.exec(h);
    if (!m) return res.status(401).json({ error: "Missing bearer token" });
    const payload = verifyJwt(m[1]);
    const user = await users.findById(payload.sub);
    if (!user) return res.status(401).json({ error: "User not found" });
    req.user = users.publicUser(user);
    req.token = m[1];
    next();
  } catch (e) {
    return res.status(401).json({ error: "Invalid or expired token" });
  }
}
