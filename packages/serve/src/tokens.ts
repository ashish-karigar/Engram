import { createHash, randomBytes } from "node:crypto";
import { ensureActor, newId, type Actor, type ActorKind, type Db } from "@engram/core";

const hash = (raw: string) => createHash("sha256").update(raw).digest("hex");

/** Issue a token for an agent (or the UI). The raw value is returned once and never stored. */
export function issueToken(db: Db, kind: ActorKind, name: string): { token: string; actor: Actor } {
  const actor = ensureActor(db, kind, name);
  const token = `egm_${randomBytes(24).toString("base64url")}`;
  db.prepare("INSERT INTO tokens (id, actor_id, token_hash, created_at) VALUES (?, ?, ?, ?)").run(
    newId("tok"),
    actor.id,
    hash(token),
    new Date().toISOString(),
  );
  return { token, actor };
}

export function verifyToken(db: Db, raw: string | undefined): Actor | null {
  if (!raw) return null;
  const row = db
    .prepare(
      `SELECT a.id, a.kind, a.name FROM tokens t JOIN actors a ON a.id = t.actor_id
       WHERE t.token_hash = ? AND t.revoked_at IS NULL`,
    )
    .get(hash(raw)) as Actor | undefined;
  return row ? { ...row } : null;
}

export function revokeTokens(db: Db, actorId: string): void {
  db.prepare("UPDATE tokens SET revoked_at = ? WHERE actor_id = ? AND revoked_at IS NULL").run(
    new Date().toISOString(),
    actorId,
  );
}
