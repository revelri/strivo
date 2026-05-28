/// <reference types="@cloudflare/workers-types" />

/**
 * Worker bindings + env vars + secrets.
 *
 * Keep this file as the single source of truth for what's injected at
 * request time. Match wrangler.toml `[vars]` + `wrangler secret put`
 * names exactly.
 */
export interface Env {
  // D1 binding (wrangler.toml)
  LICENCE_DB: D1Database;
  RATE_LIMITER: RateLimit;

  // Public vars
  PUBLIC_BASE_URL: string;
  TRIAL_DURATION_HOURS: string;
  REFRESH_INTERVAL_HOURS: string;
  LEMONSQUEEZY_STORE_ID: string;
  LEMONSQUEEZY_PRODUCT_ID: string;

  // Secrets
  JWT_PRIVATE_KEY: string;
  LEMONSQUEEZY_WEBHOOK_SECRET: string;
  LEMONSQUEEZY_API_KEY: string;
  RESEND_API_KEY: string;
  RESEND_FROM: string;
}
