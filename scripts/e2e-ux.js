// Browser e2e for the dashboard UX: wayfinding, env secrecy, logs, teams,
// validation and mobile navigation.
const { chromium } = require("playwright");

(async () => {
  const [url, statePath] = process.argv.slice(2);
  const browser = await chromium.launch();
  const ctx = await browser.newContext({
    storageState: statePath,
    viewport: { width: 1200, height: 900 },
  });
  const page = await ctx.newPage();
  const fail = (m) => {
    console.error("FAIL:", m);
    process.exit(1);
  };
  page.on("dialog", (d) => d.accept());

  // ---- wayfinding -------------------------------------------------------
  await page.goto(url + "/dashboard", { waitUntil: "domcontentloaded" });
  const title = await page.title();
  if (!/E2E Team · Projects · dok$/.test(title)) fail("dashboard title: " + title);
  let body = await page.textContent("body");
  if (!body.includes("uxapp")) fail("dashboard card for uxapp missing");
  if (!body.includes("uxapp.127.0.0.1")) fail("card host missing");
  const active = await page.locator("aside a.on").allTextContents();
  if (!active.includes("E2E Team")) fail("team not marked active in sidebar");

  // Invalid domain is an inline 422, not a 404/500.
  await page.goto(url + "/dashboard/project/uxapp/domains");
  await page.fill("#new-domain", "nodot");
  await page.click('button:has-text("Add domain")');
  if (!(await page.textContent("body")).includes("Enter a hostname with a dot"))
    fail("invalid domain did not show an inline 422 message");
  if ((await page.title()).includes("Not found")) fail("invalid domain was a 404 page");

  // ---- environment editor: secrets are write-only ----------------------
  await page.goto(url + "/dashboard/project/uxapp/env");
  await page.fill("#new-key", "SECRET_TOKEN");
  await page.fill("#new-value", "s3cret-value");
  await page.check("#env-add input[name=secret]");
  await page.click('#env-add button:has-text("Add variable")');
  await page.waitForFunction(() => document.body.innerText.includes("saved in production"));
  body = await page.content();
  if (body.includes("s3cret-value")) fail("secret value leaked into the page");
  if (!body.includes("••••••••")) fail("secret is not masked in the table");
  const bulk = await page.inputValue("#bulk");
  if (!bulk.includes("SECRET_TOKEN=") || bulk.includes("s3cret-value"))
    fail("secret leaked into the bulk textarea");

  const secretRow = page.locator("form.env-row", { hasText: "SECRET_TOKEN" });
  await secretRow.locator("input[name=value]").fill("s3cret-rotated");
  await secretRow.locator('button:has-text("Save")').click();
  await page.waitForFunction(() => document.body.innerText.includes("SECRET_TOKEN saved"));
  if ((await page.content()).includes("s3cret-rotated")) fail("rotated secret leaked");

  // Explicitly making it visible is allowed, but only after unchecking.
  const rotatedRow = page.locator("form.env-row", { hasText: "SECRET_TOKEN" });
  await rotatedRow.locator("input[name=secret]").uncheck();
  await rotatedRow.locator('button:has-text("Save")').click();
  await page.waitForFunction(() => document.body.innerText.includes("SECRET_TOKEN saved"));
  if (!(await page.content()).includes("s3cret-rotated")) fail("explicit declassification did not apply");

  // Remove is a destructive action with a confirmation.
  const removeRow = page.locator("form.env-row", { hasText: "SECRET_TOKEN" });
  await removeRow.locator('button:has-text("Remove")').click();
  await page.waitForFunction(() => document.body.innerText.includes("removed from production"));
  if (await page.locator("form.env-row", { hasText: "SECRET_TOKEN" }).count())
    fail("removed key still present");

  await page.fill("#new-key", "PUBLIC_VAR");
  await page.fill("#new-value", "hello-public");
  await page.click('#env-add button:has-text("Add variable")');
  await page.waitForFunction(() => document.body.innerText.includes("PUBLIC_VAR saved"));
  if (!(await page.content()).includes("hello-public")) fail("non-secret value not shown");

  // ---- logs -------------------------------------------------------------
  await page.goto(url + "/dashboard/project/uxapp/logs");
  body = await page.textContent("body");
  if (!body.includes("Recent requests")) fail("access log table missing");
  if (!body.includes("Static app")) fail("static runtime empty state missing");
  if (!(await page.locator("#deploy-select").count())) fail("deploy selector missing");
  await page.goto(url + "/dashboard/project/uxapp/logs?deploy=");
  if (!(await page.textContent("body")).includes("Build log")) fail("build log section missing");

  // ---- teams ------------------------------------------------------------
  await page.goto(url + "/dashboard/team");
  body = await page.textContent("body");
  if (!body.includes("E2E User")) fail("member list missing");
  if ((await page.locator("#invite-create select").inputValue()) !== "member")
    fail("invite role should default to member");
  await page.click('#invite-create button:has-text("Create invite link")');
  body = await page.textContent("body");
  if (!body.includes("/join/")) fail("invite link not shown after creation");
  if (!body.includes("active")) fail("invite not listed as active");
  if (!(await page.locator('button:has-text("Revoke")').count())) fail("revoke button missing");
  if (!(await page.locator('button:has-text("Copy link")').count())) fail("copy invite button missing");
  const invite = (body.match(/\/join\/[a-z0-9]+/) || [])[0];
  if (!invite) fail("invite URL not found");
  await page.goto(url + invite);
  if (!(await page.textContent("body")).includes("Join E2E Team"))
    fail("signed-in join page missing the team name");
  await page.click('button:has-text("Join E2E Team")');
  await page.waitForURL("**/dashboard**");
  body = await page.textContent("body");
  if (!body.includes("already a member")) fail("existing member invite was not refused");
  // An existing member must not burn the single-use invite.
  await page.goto(url + "/dashboard/team");
  body = await page.textContent("body");
  if (!body.includes("active")) fail("existing member consumed the invite");

  await page.goto(url + "/join/not-a-real-token");
  if (!(await page.textContent("body")).includes("not valid"))
    fail("invalid invite did not fail immediately");
  if (await page.locator("#go").count()) fail("invalid invite still renders the passkey form");

  // ---- proxy and settings forms save with feedback ----------------------
  await page.goto(url + "/dashboard/project/uxapp/proxy");
  if (!(await page.locator("#proxy-headers").count())) fail("proxy form missing");
  await page.click('button:has-text("Save proxy settings")');
  await page.waitForFunction(() => document.body.innerText.includes("Proxy settings saved"));
  await page.goto(url + "/dashboard/project/uxapp/settings");
  body = await page.textContent("body");
  if (!body.includes("Danger zone")) fail("danger zone missing");
  if (!body.includes("Removes 1 deployments")) fail("delete confirmation summary missing: " + body.slice(0, 400));
  await page.click('button:has-text("Save settings")');
  await page.waitForFunction(() => document.body.innerText.includes("Settings saved."));

  // ---- webhook settings are in the dashboard ----------------------------
  if (!(await page.locator("#hook-branch").count())) fail("webhook branch field missing");
  await page.fill("#hook-branch", "main");
  await page.fill("#hook-secret", "0123456789abcdef");
  await page.click('button:has-text("Save webhook")');
  await page.waitForFunction(() => document.body.innerText.includes("Webhook settings saved."));

  // ---- personal API tokens are self-service ----------------------------
  await page.goto(url + "/dashboard/account");
  if (!(await page.locator("#token-name").count())) fail("account token form missing");
  await page.fill("#token-name", "e2e-token");
  await page.click('button:has-text("Create token")');
  await page.waitForFunction(() => document.body.innerText.includes("shown only once"));
  const newToken = (await page.textContent("#new-token")).trim();
  if (!/^[a-f0-9]{64}$/.test(newToken)) fail("new token is malformed: " + newToken);
  await page.goto(url + "/dashboard/account");
  if (await page.locator("#new-token").count()) fail("token shown again after reload");
  if (!(await page.textContent("body")).includes("e2e-token")) fail("token not listed");
  await page.click('form[action="/dashboard/tokens/revoke"] button');
  await page.waitForFunction(() => document.body.innerText.includes("Token revoked."));

  // ---- certificates are managed by the external proxy in this mode -----
  await page.goto(url + "/dashboard/project/uxapp/domains");
  const domainsText = await page.textContent("body");
  if (!domainsText.includes("external proxy")) fail("certificate table should say external proxy");
  if (await page.locator('form[action="/dashboard/certificates/renew"] button').count())
    fail("renew button should be hidden when edge is off");

  // ---- mobile drawer and DOM order -------------------------------------
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto(url + "/dashboard/project/uxapp/deployments");
  const firstTag = await page.evaluate(
    () => document.querySelector(".layout").firstElementChild.tagName
  );
  if (firstTag !== "MAIN") fail("main is not first in the DOM: " + firstTag);
  if (!(await page.locator("button.menu").isVisible())) fail("mobile menu button hidden");
  await page.click("button.menu");
  if (!(await page.evaluate(() => document.body.classList.contains("nav-open"))))
    fail("mobile drawer did not open");
  const box = await page.locator('aside button:has-text("Sign out")').boundingBox();
  if (!box || box.height < 40) fail("mobile tap target too small");
  await page.keyboard.press("Escape");
  if (await page.evaluate(() => document.body.classList.contains("nav-open")))
    fail("Escape did not close the drawer");
  if ((await page.getAttribute("button.menu", "aria-expanded")) !== "false")
    fail("Escape left aria-expanded=true");
  if (!(await page.evaluate(() => document.activeElement?.classList.contains("menu"))))
    fail("focus did not return to the menu button");

  await browser.close();
  console.log("ux: OK");
})().catch((e) => {
  console.error("FAIL:", e);
  process.exit(1);
});
