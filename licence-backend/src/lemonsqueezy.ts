/**
 * Lemon Squeezy integration:
 *   - verifyWebhook: HMAC-SHA256 over the raw body, base16 hex.
 *   - validateLicenceKey: hit the LS Licence API to confirm an
 *     activation key belongs to OUR store + product and isn't disabled.
 */
import type { Env } from "./env";

export interface LemonLicenceCheck {
  ok: boolean;
  email?: string;
  reason?: string;
}

/** Lemon Squeezy webhook signature uses HMAC-SHA256 hex. */
export async function verifyWebhook(
  env: Env,
  body: string,
  signature: string | null,
): Promise<boolean> {
  if (!signature) return false;
  const key = await crypto.subtle.importKey(
    "raw",
    new TextEncoder().encode(env.LEMONSQUEEZY_WEBHOOK_SECRET),
    { name: "HMAC", hash: "SHA-256" },
    false,
    ["sign"],
  );
  const mac = new Uint8Array(
    await crypto.subtle.sign("HMAC", key, new TextEncoder().encode(body)),
  );
  const expected = Array.from(mac, (b) => b.toString(16).padStart(2, "0")).join("");
  // constant-time-ish: same length and identical
  if (expected.length !== signature.length) return false;
  let diff = 0;
  for (let i = 0; i < expected.length; i++) diff |= expected.charCodeAt(i) ^ signature.charCodeAt(i);
  return diff === 0;
}

/** Validate that a licence key was issued by our store/product. */
export async function validateLicenceKey(
  env: Env,
  key: string,
): Promise<LemonLicenceCheck> {
  const resp = await fetch("https://api.lemonsqueezy.com/v1/licenses/validate", {
    method: "POST",
    headers: {
      Accept: "application/vnd.api+json",
      "Content-Type": "application/x-www-form-urlencoded",
    },
    body: new URLSearchParams({ license_key: key }).toString(),
  });
  if (!resp.ok) return { ok: false, reason: `LS validate HTTP ${resp.status}` };
  const j = (await resp.json()) as {
    valid: boolean;
    license_key?: { status: string };
    meta?: { store_id: number; product_id: number; customer_email?: string };
  };
  if (!j.valid) return { ok: false, reason: "lemon: invalid key" };
  if (j.license_key?.status !== "active") {
    return { ok: false, reason: `lemon: status ${j.license_key?.status}` };
  }
  if (env.LEMONSQUEEZY_STORE_ID && String(j.meta?.store_id) !== env.LEMONSQUEEZY_STORE_ID) {
    return { ok: false, reason: "lemon: wrong store" };
  }
  if (env.LEMONSQUEEZY_PRODUCT_ID && String(j.meta?.product_id) !== env.LEMONSQUEEZY_PRODUCT_ID) {
    return { ok: false, reason: "lemon: wrong product" };
  }
  return { ok: true, email: j.meta?.customer_email };
}
