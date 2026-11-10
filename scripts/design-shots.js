// Capture the design-review screenshots for the dashboard.
// Usage: NODE_PATH=/tmp/dok-e2e/node_modules node scripts/design-shots.js <url> <storage-state>
const { chromium } = require("playwright");
const fs = require("fs");
const path = require("path");

const BASE = process.argv[2] || "http://localhost:18092";
const STATE = process.argv[3] || "";
const OUT = path.join(__dirname, "..", "docs", "design", "visual");

(async () => {
  fs.mkdirSync(OUT, { recursive: true });
  const browser = await chromium.launch();
  const errors = [];

  async function shot(ctx, urlPath, file) {
    const page = await ctx.newPage();
    page.on("pageerror", (e) => errors.push(`${file}: ${e.message}`));
    await page.goto(BASE + urlPath, { waitUntil: "networkidle" });
    await page.waitForTimeout(450);
    await page.screenshot({ path: path.join(OUT, file) });
    await page.close();
  }

  const desktop = await browser.newContext({
    storageState: STATE || undefined,
    viewport: { width: 1440, height: 900 },
  });
  await shot(desktop, "/dashboard", "dashboard-populated.png");
  await shot(desktop, "/dashboard?team=studio", "dashboard-empty.png");
  await shot(desktop, "/dashboard/project/landing", "project-overview.png");
  await shot(desktop, "/dashboard/project/landing/domains", "domains.png");
  await shot(desktop, "/dashboard/project/landing/logs", "logs.png");
  await shot(desktop, "/dashboard/project/landing/env", "environment.png");
  await shot(desktop, "/dashboard/team", "team.png");
  await shot(desktop, "/dashboard/account", "account.png");

  const mobile = await browser.newContext({
    storageState: STATE || undefined,
    viewport: { width: 390, height: 844 },
    deviceScaleFactor: 2,
    isMobile: true,
    hasTouch: true,
  });
  await shot(mobile, "/dashboard", "mobile.png");
  await shot(mobile, "/dashboard/project/landing", "mobile-project.png");

  const anon = await browser.newContext({ viewport: { width: 1440, height: 900 } });
  await shot(anon, "/", "login.png");
  await shot(anon, "/join/not-a-real-token", "invite-invalid.png");

  await browser.close();
  if (errors.length) {
    console.error("page errors:\n" + errors.join("\n"));
    process.exit(1);
  }
  console.log(`design shots: OK -> ${OUT}`);
})().catch((e) => {
  console.error("FAIL:", e);
  process.exit(1);
});
