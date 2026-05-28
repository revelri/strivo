/**
 * strivo-licence — Worker entry.
 *
 * Five endpoints (see README). Every route is hand-written; no
 * framework, no router. Routes are matched by method + path in a
 * single switch. Lemon Squeezy webhooks land at /webhook/lemonsqueezy.
 *
 * Response envelope mirrors RFC 9457 Problem Details on errors so the
 * StriVo client's existing problem-decoder works.
 */
import type { Env } from "./env";
import { signEs256, nowSecs } from "./jwt";
import * as db from "./db";
import * as lemon from "./lemonsqueezy";
import * as mail from "./resend";

export default {
  async fetch(req: Request, env: Env): Promise<Response> {
    const url = new URL(req.url);
    try {
      switch (`${req.method} ${url.pathname}`) {
        case "GET /health":
          return json({ ok: true, ts: new Date().toISOString() });
        case "POST /activate":
          return await activate(req, env);
        case "POST /refresh":
          return await refresh(req, env);
        case "POST /trial":
          return await trial(req, env);
        case "POST /webhook/lemonsqueezy":
          return await webhook(req, env);
        default:
          return problem(404, "Not Found", "no route matches");
      }
    } catch (e) {
      console.error(e);
      return problem(500, "Internal Server Error", (e as Error).message);
    }
  },
} satisfies ExportedHandler<Env>;

// ── /activate ────────────────────────────────────────────────────────
//
// Body: { "licence_key": "...", "machine_hash": "hex sha256" }
// Validates the key against Lemon Squeezy, binds it to the machine,
// issues a 72h-expiry JWT. Repeated calls with the same (key, machine)
// just touch last_refreshed and re-sign.

async function activate(req: Request, env: Env): Promise<Response> {
  if (await rateLimit(req, env, "activate")) return problem(429, "Too Many Requests", "slow down");
  const body = await safeJson<{ licence_key?: string; machine_hash?: string }>(req);
  if (!body || !body.licence_key || !body.machine_hash) {
    return problem(400, "Bad Request", "licence_key and machine_hash required");
  }

  // First-call validation: only call Lemon Squeezy on the first
  // activation for this (key, machine) pair. Re-activations just
  // re-sign — saves an LS request per refresh.
  const existing = await db.findLicence(env, body.licence_key, body.machine_hash);
  let row = existing;
  if (!existing) {
    const lemonResult = await lemon.validateLicenceKey(env, body.licence_key);
    if (!lemonResult.ok) return problem(403, "Forbidden", lemonResult.reason ?? "invalid licence");
    const nowIso = new Date().toISOString();
    row = await db.createLicence(env, {
      licence_key: body.licence_key,
      machine_hash: body.machine_hash,
      tier: "pro",
      email: lemonResult.email ?? null,
      activated_at: nowIso,
      last_refreshed: nowIso,
      expires_at: null,
      revoked_at: null,
    });
  } else {
    if (existing.revoked_at) return problem(403, "Forbidden", "licence revoked");
    await db.touchRefreshed(env, existing.id, new Date().toISOString());
  }
  return json({ token: await mintToken(env, row!), tier: "pro" });
}

// ── /refresh ────────────────────────────────────────────────────────
//
// Client calls this every REFRESH_INTERVAL_HOURS (default 72) while
// online. Server re-signs the token unless the licence has been
// revoked. Body echoes machine_hash so we don't trust the JWT's
// embedded value alone.

async function refresh(req: Request, env: Env): Promise<Response> {
  const body = await safeJson<{ licence_key?: string; machine_hash?: string }>(req);
  if (!body || !body.licence_key || !body.machine_hash) {
    return problem(400, "Bad Request", "licence_key and machine_hash required");
  }
  const row = await db.findLicence(env, body.licence_key, body.machine_hash);
  if (!row) return problem(404, "Not Found", "no licence on file");
  if (row.revoked_at) return problem(403, "Forbidden", "licence revoked");
  await db.touchRefreshed(env, row.id, new Date().toISOString());
  return json({ token: await mintToken(env, row), tier: row.tier });
}

// ── /trial ──────────────────────────────────────────────────────────
//
// Body: { "machine_hash": "..." }. Issues a 3-day token if this
// machine has never claimed a trial. One-shot.

async function trial(req: Request, env: Env): Promise<Response> {
  if (await rateLimit(req, env, "trial")) return problem(429, "Too Many Requests", "slow down");
  const body = await safeJson<{ machine_hash?: string }>(req);
  if (!body || !body.machine_hash) {
    return problem(400, "Bad Request", "machine_hash required");
  }
  if (await db.trialAlreadyClaimed(env, body.machine_hash)) {
    return problem(409, "Conflict", "trial already claimed on this machine");
  }
  const trialHours = parseInt(env.TRIAL_DURATION_HOURS || "72", 10);
  const now = new Date();
  const expires = new Date(now.getTime() + trialHours * 3600 * 1000);
  const row = await db.createLicence(env, {
    licence_key: "",
    machine_hash: body.machine_hash,
    tier: "trial",
    email: null,
    activated_at: now.toISOString(),
    last_refreshed: now.toISOString(),
    expires_at: expires.toISOString(),
    revoked_at: null,
  });
  await db.recordTrialClaim(env, body.machine_hash, now.toISOString());
  return json({ token: await mintToken(env, row), tier: "trial", expires_at: expires.toISOString() });
}

// ── /webhook/lemonsqueezy ───────────────────────────────────────────
//
// Lemon Squeezy POSTs every event with `X-Signature: <hex hmac>` over
// the raw body. We care about refunds + disputes for revocation, and
// `order_created` for the receipt email.

async function webhook(req: Request, env: Env): Promise<Response> {
  const sig = req.headers.get("X-Signature");
  const raw = await req.text();
  if (!(await lemon.verifyWebhook(env, raw, sig))) {
    return problem(401, "Unauthorized", "bad signature");
  }
  const payload = JSON.parse(raw) as {
    meta?: { event_name?: string; webhook_id?: string };
    data?: { id: string; attributes?: Record<string, unknown> };
  };
  const eventId = payload.meta?.webhook_id ?? payload.data?.id ?? "";
  const kind = payload.meta?.event_name ?? "unknown";
  const fresh = await db.recordWebhook(env, eventId, kind, raw, new Date().toISOString());
  if (!fresh) return json({ ok: true, duplicate: true });

  switch (kind) {
    case "order_created": {
      const attrs = (payload.data?.attributes ?? {}) as {
        first_order_item?: { license_key?: { key?: string } };
        user_email?: string;
      };
      const key = attrs.first_order_item?.license_key?.key;
      const email = attrs.user_email;
      if (key && email) await mail.sendPurchaseReceipt(env, email, key);
      break;
    }
    case "subscription_payment_refunded":
    case "order_refunded": {
      const attrs = (payload.data?.attributes ?? {}) as {
        first_order_item?: { license_key?: { key?: string } };
        user_email?: string;
      };
      const key = attrs.first_order_item?.license_key?.key;
      const email = attrs.user_email;
      if (key) await db.revokeByLicenceKey(env, key, new Date().toISOString());
      if (email) await mail.sendRefundNotice(env, email);
      break;
    }
    default:
      break;
  }
  return json({ ok: true });
}

// ── helpers ─────────────────────────────────────────────────────────

/** Mint a JWT for a licence row. Token TTL = REFRESH_INTERVAL_HOURS. */
async function mintToken(env: Env, row: db.LicenceRow): Promise<string> {
  const ttlSecs = parseInt(env.REFRESH_INTERVAL_HOURS || "72", 10) * 3600;
  const payload: Record<string, unknown> = {
    iss: env.PUBLIC_BASE_URL,
    sub: row.machine_hash,
    tier: row.tier,
    iat: nowSecs(),
    exp: nowSecs() + ttlSecs,
  };
  if (row.expires_at) payload.licence_exp = row.expires_at;
  return signEs256(env.JWT_PRIVATE_KEY, payload);
}

/** Returns true when the request should be rejected. */
async function rateLimit(req: Request, env: Env, key: string): Promise<boolean> {
  // Workers Rate Limiting binding. Key by client IP + route so a single
  // abusive box can't burn the limit for everyone.
  const ip = req.headers.get("CF-Connecting-IP") ?? "anon";
  const res = await env.RATE_LIMITER.limit({ key: `${ip}:${key}` });
  return !res.success;
}

async function safeJson<T>(req: Request): Promise<T | null> {
  try {
    return (await req.json()) as T;
  } catch {
    return null;
  }
}

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  });
}

function problem(status: number, title: string, detail: string): Response {
  return new Response(
    JSON.stringify({ type: "about:blank", title, status, detail, instance: null }),
    { status, headers: { "Content-Type": "application/problem+json" } },
  );
}
