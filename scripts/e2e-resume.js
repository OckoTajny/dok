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

  await browser.close();
  console.log("resume: OK");
})().catch((e) => {
  console.error("FAIL:", e);
  process.exit(1);
});
