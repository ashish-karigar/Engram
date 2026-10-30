use engram_core::db::{new_id, now};
use engram_core::events::{Actor, ensure_actor};
use engram_core::rusqlite::OptionalExtension;
use engram_core::{Connection, Result};
use sha2::{Digest, Sha256};
use uuid::Uuid;

fn hash(raw: &str) -> String {
    hex::encode(Sha256::digest(raw))
}

/// Issue a token for an agent or the UI. The raw value is returned once and only its hash is stored.
pub fn issue_token(conn: &Connection, kind: &str, name: &str) -> Result<(String, Actor)> {
    let actor = ensure_actor(conn, kind, name)?;
    let token = format!("egm_{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    conn.execute(
        "INSERT INTO tokens (id, actor_id, token_hash, created_at) VALUES (?, ?, ?, ?)",
        [new_id("tok"), actor.id.clone(), hash(&token), now()],
    )?;
    Ok((token, actor))
}

pub fn verify_token(conn: &Connection, raw: &str) -> Result<Option<Actor>> {
    Ok(conn
        .query_row(
            "SELECT a.id, a.kind, a.name FROM tokens t JOIN actors a ON a.id = t.actor_id
             WHERE t.token_hash = ? AND t.revoked_at IS NULL",
            [hash(raw)],
            |r| Ok(Actor { id: r.get(0)?, kind: r.get(1)?, name: r.get(2)? }),
        )
        .optional()?)
}

pub fn revoke_tokens(conn: &Connection, actor_id: &str) -> Result<()> {
    conn.execute("UPDATE tokens SET revoked_at = ? WHERE actor_id = ? AND revoked_at IS NULL", [now(), actor_id.to_string()])?;
    Ok(())
}
