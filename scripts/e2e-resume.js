const { chromium } = require("playwright");

(async () => {
  const [url, statePath] = process.argv.slice(2);
  const browser = await chromium.launch();
  const ctx = await browser.newContext({ storageState: statePath });
  const page = await ctx.newPage();

  const fail = (m) => {
    console.error("FAIL:", m);
    process.exit(1);
  };

  // Session must survive a server restart (stored in sqlite, not memory).
  const resp = await page.goto(url + "/dashboard", { waitUntil: "domcontentloaded" });
  if (!resp || resp.status() !== 200) fail("dashboard after restart: " + (resp && resp.status()));
  const text = await page.textContent("body");
  if (!text.includes("hello")) fail("app list missing after restart");

  const csrf = await page.getAttribute('input[name="csrf"]', "value");
  if (!csrf) fail("no csrf token in dashboard");

  const bad = await page.evaluate(() =>
    fetch("/dashboard/redeploy/hello", { method: "POST", body: "csrf=nope" }).then((r) => r.status)
  );
  if (bad !== 403) fail("expected 403 without csrf, got " + bad);

  const good = await page.evaluate(
    (c) =>
      fetch("/dashboard/redeploy/nope", {
        method: "POST",
        headers: { "content-type": "application/x-www-form-urlencoded" },
        body: "csrf=" + c,
      }).then((r) => r.status),
    csrf
  );
  if (good === 403) fail("valid csrf rejected");

  const logs = await page.goto(url + "/dashboard/logs/hello", { waitUntil: "domcontentloaded" });
  if (!logs || logs.status() !== 200) fail("logs page: " + (logs && logs.status()));

  // Every project tab must render, including the edge/certificate views.
  const tabs = {
    deployments: "Deployments",
    env: "Environment",
    domains: "Certificates",
    proxy: "Basic auth",
    settings: "Danger zone",
  };
  for (const [tab, needle] of Object.entries(tabs)) {
    const resp2 = await page.goto(url + "/dashboard/project/hello/" + tab, {
      waitUntil: "domcontentloaded",
    });
    if (!resp2 || resp2.status() !== 200) fail(tab + " tab: " + (resp2 && resp2.status()));
    const body = await page.textContent("body");
    if (!body.includes(needle)) fail(tab + " tab missing " + needle);
  }

  // Creating a project from the dashboard writes through the CSRF-protected form.
  await page.goto(url + "/dashboard", { waitUntil: "domcontentloaded" });
  await page.fill('input[name="name"]', "e2e-new");
  await page.click("text=New project");
  await page.waitForSelector("text=e2e-new", { timeout: 10000 });

  await browser.close();
  console.log("resume: OK");
})().catch((e) => {
  console.error("FAIL:", e);
  process.exit(1);
});
