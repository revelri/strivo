import { test, expect } from "@playwright/test";

// W7 — critical-path smoke journeys against the real SPA + mock backend.

test("login page renders and accepts a key", async ({ page }) => {
  await page.goto("/app#/login");
  await expect(page.locator("#login-form")).toBeVisible();
  await page.locator("#api-key").fill("test-key");
  await page.locator("#login-form button[type=submit]").click();
  // On success the SPA leaves the login route for the library.
  await expect(page.locator(".leftrail")).toBeVisible();
});

test("library shows live + offline channels", async ({ page }) => {
  await page.goto("/app#/library");
  await expect(page.getByText("LIVE NOW")).toBeVisible();
  await expect(page.getByText("Live Channel")).toBeVisible();
  await expect(page.getByText("Offline Channel")).toBeVisible();
});

test("bulk-download button is present and fires a request", async ({ page }) => {
  await page.goto("/app#/library");
  const reqs: string[] = [];
  page.on("request", (r) => {
    if (r.url().includes("/bulk")) reqs.push(r.url());
  });
  await page.getByRole("button", { name: /Bulk DL/ }).first().click();
  await expect.poll(() => reqs.length).toBeGreaterThan(0);
});

test("command palette opens with Ctrl+K and navigates", async ({ page }) => {
  await page.goto("/app#/library");
  await page.keyboard.press("Control+k");
  await expect(page.locator("#cmdk.open")).toBeVisible();
  await page.locator("#cmdk-input").fill("recordings");
  await page.keyboard.press("Enter");
  await expect(page).toHaveURL(/#\/recordings/);
});

test("recordings grid filters and sorts", async ({ page }) => {
  await page.goto("/app#/recordings");
  await expect(page.locator(".recordings-table tbody tr")).toHaveCount(3);
  // Filter by channel name.
  await page.locator("#rec-filter").fill("bravo");
  await expect(page.locator(".recordings-table tbody tr")).toHaveCount(1);
  await page.locator("#rec-filter").fill("");
  // Sort by Channel ascending — first row should be Alpha.
  await page.locator('th[data-sort="channel"]').click();
  await expect(page.locator(".recordings-table tbody tr").first()).toContainText("Alpha");
});

test("patreon route renders without a daemon", async ({ page }) => {
  await page.goto("/app#/patreon");
  await expect(page.getByRole("heading", { name: "Patreon" })).toBeVisible();
});
