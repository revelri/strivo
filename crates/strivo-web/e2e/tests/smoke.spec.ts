import { test, expect } from "@playwright/test";

// W7 — critical-path smoke journeys against the real SPA + mock backend.
// Updated for the TUI-style 3-pane redesign: channel-list left rail,
// channel-detail center, recordings dashboard, no Activity surface.

test("login page renders and accepts a key", async ({ page }) => {
  await page.goto("/app#/login");
  await expect(page.locator("#login-form")).toBeVisible();
  await page.locator("#api-key").fill("test-key");
  await page.locator("#login-form button[type=submit]").click();
  // On success the SPA leaves login for the home chrome (channel rail).
  await expect(page.locator("#channel-list")).toBeVisible();
});

test("left rail lists channels, live first", async ({ page }) => {
  await page.goto("/app#/library");
  await expect(page.locator("#channel-list")).toBeVisible();
  await expect(page.getByText("Live Channel")).toBeVisible();
  await expect(page.getByText("Offline Channel")).toBeVisible();
  // LIVE section header appears for the live channel.
  await expect(page.locator(".ch-section-title", { hasText: "LIVE" })).toBeVisible();
});

test("recordings dashboard shows the three rows by default", async ({ page }) => {
  await page.goto("/app#/library");
  await expect(page.getByRole("heading", { name: "In progress" })).toBeVisible();
  await expect(page.getByRole("heading", { name: "Recent" })).toBeVisible();
  await expect(page.getByRole("heading", { name: "Upcoming" })).toBeVisible();
});

test("clicking a YouTube channel shows detail with streams + uploads", async ({ page }) => {
  await page.goto("/app#/library");
  await page.locator(".ch-row", { hasText: "Live Channel" }).click();
  await expect(page.locator(".cd-name")).toHaveText("Live Channel");
  // VOD lists arrive over SSE (mock pushes one LiveBroadcast + one Upload).
  await expect(page.getByText("Yesterday's livestream")).toBeVisible();
  await expect(page.getByText("How I edit my videos")).toBeVisible();
  await expect(page.locator(".cd-section-title", { hasText: "Recent live streams" })).toBeVisible();
  await expect(page.locator(".cd-section-title", { hasText: "Recent uploads" })).toBeVisible();
});

test("patreon creators appear in the left rail (seeded from /patreon)", async ({ page }) => {
  await page.goto("/app#/library");
  // Seeded on boot from /patreon — no waiting on a poll-driven SSE event.
  await expect(page.locator(".ch-section-title", { hasText: "Patreon" })).toBeVisible();
  await expect(page.getByText("Cool Creator")).toBeVisible();
  await expect(page.locator(".ch-tier", { hasText: "Premium Tier" })).toBeVisible();
});

test("no Activity surface anywhere", async ({ page }) => {
  await page.goto("/app#/library");
  await expect(page.locator(".activity-rail")).toHaveCount(0);
  await expect(page.locator('[data-route="activity"]')).toHaveCount(0);
});

test("top-bar icon nav reaches the recordings table", async ({ page }) => {
  await page.goto("/app#/library");
  await page.locator('.topnav-link[data-route="recordings"]').click();
  await expect(page).toHaveURL(/#\/recordings/);
  await expect(page.locator(".recordings-table")).toBeVisible();
});

test("recordings density toggle + multi-select mass bar", async ({ page }) => {
  await page.goto("/app#/recordings");
  await expect(page.locator(".recordings-table")).toBeVisible();
  // Density toggle adds the compact class.
  await page.locator("#rec-density").click();
  await expect(page.locator(".recordings-table.compact")).toBeVisible();
  // Selecting a row reveals the mass-action bar.
  await page.locator(".rec-row-check").first().check();
  await expect(page.locator("#rec-massbar")).toBeVisible();
  await expect(page.locator("#rec-massbar")).toContainText("selected");
});

test("settings page renders real config sections", async ({ page }) => {
  await page.goto("/app#/settings");
  await expect(page.getByRole("heading", { name: "Settings" })).toBeVisible();
  await expect(page.getByRole("heading", { name: "Platforms" })).toBeVisible();
  await expect(page.getByRole("heading", { name: "Recording" })).toBeVisible();
});

test("system page renders health + tasks", async ({ page }) => {
  await page.goto("/app#/system");
  await expect(page.getByRole("heading", { name: "System" })).toBeVisible();
  await expect(page.getByRole("heading", { name: "Health" })).toBeVisible();
  await expect(page.locator(".sys-check").first()).toBeVisible();
  await expect(page.getByRole("heading", { name: "Backup" })).toBeVisible();
  await expect(page.locator("#backup-now")).toBeVisible();
  await expect(page.locator(".restore-backup").first()).toBeVisible();
  await expect(page.getByRole("heading", { name: "Blocklist" })).toBeVisible();
  await expect(page.locator(".unblock").first()).toBeVisible();
});

test("logs page renders with level selector and lines", async ({ page }) => {
  await page.goto("/app#/logs");
  await expect(page.getByRole("heading", { name: "Logs" })).toBeVisible();
  await expect(page.locator("#logs-level")).toBeVisible();
  await expect(page.locator("#logs-output")).toContainText("StriVo daemon starting");
});

test("schedule page renders the upcoming agenda", async ({ page }) => {
  await page.goto("/app#/schedule");
  await expect(page.getByRole("heading", { name: "Schedule" })).toBeVisible();
  await expect(page.locator(".cfg-grid")).toContainText("Alpha");
  await expect(page.locator(".agenda-time").first()).toBeVisible();
});

test("history page renders durable jobs from the DB", async ({ page }) => {
  await page.goto("/app#/history");
  await expect(page.getByRole("heading", { name: "History" })).toBeVisible();
  await expect(page.locator(".recordings-table")).toContainText("LilAggy");
  await expect(page.locator(".recordings-table")).toContainText("Finished");
});

test("add-channel wizard opens to phase 1 search", async ({ page }) => {
  await page.goto("/app#/library");
  await page.locator("#add-channel").click();
  await expect(page.locator("#add-channel-modal.open")).toBeVisible();
  await expect(page.locator("#aw-platform")).toBeVisible();
  await expect(page.locator("#aw-query")).toBeVisible();
  await expect(page.locator("#aw-search")).toBeVisible();
});

test("ARIA toast live regions are pre-created on load", async ({ page }) => {
  await page.goto("/app#/library");
  // Both regions must exist before any toast fires, for reliable SR announce.
  await expect(page.locator('.toast-region[role="status"][aria-live="polite"]')).toHaveCount(1);
  await expect(page.locator('.toast-region[role="alert"][aria-live="assertive"]')).toHaveCount(1);
  // The wrap must be non-interactive (pointer-events: none).
  const pe = await page.locator(".toast-wrap").evaluate((el) => getComputedStyle(el).pointerEvents);
  expect(pe).toBe("none");
});

test("command palette opens with Ctrl+K and navigates", async ({ page }) => {
  await page.goto("/app#/library");
  await page.keyboard.press("Control+k");
  await expect(page.locator("#cmdk.open")).toBeVisible();
  await page.locator("#cmdk-input").fill("recordings");
  await page.keyboard.press("Enter");
  await expect(page).toHaveURL(/#\/recordings/);
});
