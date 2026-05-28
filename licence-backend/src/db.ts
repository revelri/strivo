/**
 * D1 access helpers. Every query is parameterised; no string-built SQL.
 * Each helper returns the bare row shape we need at the call site so
 * routes don't have to think about D1 result envelopes.
 */
import type { Env } from "./env";

export interface LicenceRow {
  id: number;
  licence_key: string;
  machine_hash: string;
  tier: "trial" | "pro";
  email: string | null;
  activated_at: string;
  last_refreshed: string;
  expires_at: string | null;
  revoked_at: string | null;
}

export async function findLicence(
  env: Env,
  licenceKey: string,
  machineHash: string,
): Promise<LicenceRow | null> {
  const r = await env.LICENCE_DB.prepare(
    "SELECT * FROM licences WHERE licence_key = ?1 AND machine_hash = ?2",
  )
    .bind(licenceKey, machineHash)
    .first<LicenceRow>();
  return r ?? null;
}

export async function createLicence(
  env: Env,
  row: Omit<LicenceRow, "id">,
): Promise<LicenceRow> {
  const result = await env.LICENCE_DB.prepare(
    `INSERT INTO licences
       (licence_key, machine_hash, tier, email, activated_at, last_refreshed, expires_at, revoked_at)
     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
     RETURNING *`,
  )
    .bind(
      row.licence_key,
      row.machine_hash,
      row.tier,
      row.email,
      row.activated_at,
      row.last_refreshed,
      row.expires_at,
      row.revoked_at,
    )
    .first<LicenceRow>();
  if (!result) throw new Error("INSERT … RETURNING returned no row");
  return result;
}

export async function touchRefreshed(
  env: Env,
  id: number,
  iso: string,
): Promise<void> {
  await env.LICENCE_DB.prepare("UPDATE licences SET last_refreshed = ?1 WHERE id = ?2")
    .bind(iso, id)
    .run();
}

export async function revokeByLicenceKey(
  env: Env,
  licenceKey: string,
  iso: string,
): Promise<void> {
  await env.LICENCE_DB.prepare(
    "UPDATE licences SET revoked_at = ?1 WHERE licence_key = ?2 AND revoked_at IS NULL",
  )
    .bind(iso, licenceKey)
    .run();
}

export async function trialAlreadyClaimed(
  env: Env,
  machineHash: string,
): Promise<boolean> {
  const r = await env.LICENCE_DB.prepare(
    "SELECT 1 AS one FROM trial_claims WHERE machine_hash = ?1",
  )
    .bind(machineHash)
    .first<{ one: number }>();
  return r != null;
}

export async function recordTrialClaim(
  env: Env,
  machineHash: string,
  iso: string,
): Promise<void> {
  await env.LICENCE_DB.prepare(
    "INSERT INTO trial_claims (machine_hash, claimed_at) VALUES (?1, ?2)",
  )
    .bind(machineHash, iso)
    .run();
}

export async function recordWebhook(
  env: Env,
  eventId: string,
  kind: string,
  payload: string,
  iso: string,
): Promise<boolean> {
  // INSERT OR IGNORE — Lemon Squeezy can retry; we want idempotency.
  const r = await env.LICENCE_DB.prepare(
    "INSERT OR IGNORE INTO webhook_events (event_id, kind, payload, received_at) VALUES (?1, ?2, ?3, ?4)",
  )
    .bind(eventId, kind, payload, iso)
    .run();
  // r.meta.changes is 0 when the event was a duplicate.
  return (r.meta as { changes?: number }).changes !== 0;
}
