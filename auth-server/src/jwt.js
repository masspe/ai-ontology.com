// SPDX-License-Identifier: AGPL-3.0-or-later
import jwt from "jsonwebtoken";
import { config } from "./config.js";

export function signJwt(user) {
  return jwt.sign(
    { sub: user.id, email: user.email, name: user.name },
    config.jwtSecret,
    { expiresIn: config.jwtExpiresIn, issuer: "ai-ontology", audience: "web" },
  );
}

export function verifyJwt(token) {
  return jwt.verify(token, config.jwtSecret, { issuer: "ai-ontology", audience: "web" });
}
