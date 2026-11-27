#!/usr/bin/env node
// Screenshots every screen of the browser preview (src/preview) in each of its states, in light and dark, at the
// smallest window (980 x 640) and a large one (1600 x 1000), at 2x. File names stay the same from run to run, so two
// runs pair up as before and after: <area>--<screen>-<state>--<theme>--<w>x<h>.png, with a manifest.json listing each.
//
//   node scripts/preview-shots.mjs [--out <dir>] [--only <text>] [--url <dev server>]
//
// --only keeps the shots whose file name contains the text ("inbox--", "--dark--", "org--panel"); it replaces just
// those in the folder and in its manifest.
// It reuses a dev server already serving this folder (on 1420), else starts one. It drives Google Chrome with
// playwright-core from $PLAYWRIGHT_CORE, or wherever Node finds it (npm install --no-save playwright-core); it adds
// nothing to this project's dependencies.
//
// The browser's clock stands still at one moment (CLOCK, Sydney time, en-AU), so "2h ago", "Today" and the day
// headings come out the same whenever the shots are taken. Timers still run, so live work still streams.
import { spawn } from "node:child_process";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const SIZES = [[980, 640], [1600, 1000]];
const THEMES = ["light", "dark"];
const WORKERS = 4;
/** A Thursday afternoon: the invented office's times are all relative to this. */
const CLOCK = new Date("2026-11-05T14:30:00+11:00");
const BROWSER = { timezoneId: "Australia/Sydney", locale: "en-AU" };

// ---------------------------------------------------------------- what to shoot

const palette = async (p) => { await p.keyboard.press("Control+k"); await p.waitForSelector(".palette, [role=dialog]"); };
/** Restart to update. While work runs it asks first: the sidebar's confirm, or the rail's second press. */
const clickUpdate = async (p) => {
  const row = p.locator(".update-row .update-btn");
  if (await row.isVisible()) {
    await row.click();
    if (await p.locator('.update .collapse[data-open="true"]').count()) await p.locator(".update-confirm .update-btn").click();
  } else {
    await p.locator(".update-mini").click();
    if (await p.locator(".update-mini-armed").count()) await p.locator(".update-mini").click();
  }
};
const askUpdate = async (p) => {
  const row = p.locator(".update-row .update-btn");
  await (await row.isVisible() ? row : p.locator(".update-mini")).click();
};
/** Onboarding's fields, found by their labels. `skip` leaves those out (a field left empty on purpose). */
const fillSetup = async (p, skip = []) => {
  const fill = async (label, value, exact = true) => { if (!skip.includes(label)) await p.getByLabel(label, { exact }).fill(value); };
  await fill("Company", "Fernbrook");
  await fill("Your name", "Maren Holt");
  await fill("Your title", "Founder", false);
  await fill("What we sell", "Booking software for small fitness studios");
  await fill("How I like to work", "Show me a draft before it goes to every studio");
};
const openOffice = (p) => p.getByRole("button", { name: /Open your office|Opening/ }).click({ force: true });
/** Settings: change a control, then give the save `ms` to answer before the shot. */
const settingsChange = (label, ms = 0) => async (p) => {
  await p.getByRole("switch", { name: label }).click({ force: true });
  if (ms) await p.waitForTimeout(ms);
};
const scrollTo = (text) => (p) => p.getByText(text, { exact: true }).first().scrollIntoViewIfNeeded();
const scrollToEnd = (p) => p.evaluate(() => {
  for (const el of document.querySelectorAll(".content *")) {
    if (el.scrollHeight > el.clientHeight + 4 && /auto|scroll/.test(getComputedStyle(el).overflowY)) el.scrollTop = el.scrollHeight;
  }
});

/** Asks a question on a card in the Inbox, then waits for its answer under the card, or with `answered` off for the
 *  employee to start on it. */
const askOnCard = (answered) => async (p) => {
  await p.locator('[data-row="summary-1:cards"]').click();
  await p.getByRole("button", { name: "Reply in words" }).click();
  await p.locator("[data-card-reply]").fill("Which studios still had double bookings?");
  await p.locator('button[data-tip^="Sends it as a question"]').click();
  const asked = p.locator(".d-asked");
  await (answered ? asked.locator(".d-report-body") : asked.getByText("is answering")).waitFor();
  await asked.scrollIntoViewIfNeeded();
};

/** Opens the first notes file a reply names in the reader beside it. */
const openNotes = (p) => p.locator(".msg-emp .md-path").first().click();

/** area, screen, state (?preview=), route, and what to do once it has settled. */
const S = (area, screen, state, hash, o = {}) => ({ area, screen, state, hash, ...o });
const SHOTS = [
  S("shell", "boot", "loading-boot", "#/org"),
  S("shell", "boot", "error-boot", "#/org"),
  S("shell", "palette", "full", "#/org", { act: palette }),
  S("shell", "palette-search", "full", "#/org", { act: async (p) => { await palette(p); await p.keyboard.type("price"); } }),
  S("shell", "sidebar-collapsed", "full", "#/inbox", { query: "sidebar=collapsed" }),
  S("shell", "sidebar-expanded", "full", "#/inbox", { query: "sidebar=expanded" }),
  S("shell", "sidebar-quiet", "quiet", "#/org", { query: "sidebar=expanded" }),
  S("shell", "chrome-mac", "full", "#/org", { query: "chrome=mac&sidebar=expanded" }),
  S("shell", "chrome-mac-rail", "full", "#/org", { query: "chrome=mac&sidebar=collapsed" }),
  S("shell", "update-confirm", "full", "#/activity", { query: "sidebar=expanded", act: askUpdate }),
  S("shell", "update-armed", "full", "#/activity", { query: "sidebar=collapsed", act: askUpdate }),
  S("shell", "palette-nothing", "full", "#/org", { act: async (p) => { await palette(p); await p.keyboard.type("zebra crossing"); } }),
  S("shell", "toasts", "full", "#/org", { query: "toasts=1" }),
  S("shell", "update-installing", "full", "#/activity", { act: clickUpdate }),
  S("shell", "update", "update-failed", "#/activity", { act: clickUpdate }),
  S("shell", "usage", "usage-limit", "#/inbox"),
  S("shell", "usage", "rate-limit", "#/inbox"),
  S("shell", "usage", "api-key", "#/inbox"),
  S("shell", "star-ask", "star-ask", "#/org", { query: "sidebar=expanded" }),
  S("shell", "star-ask-rail", "star-ask", "#/org", { query: "sidebar=collapsed" }),

  S("onboarding", "setup", "first-run", ""),
  S("onboarding", "setup-filled", "first-run", "", { act: fillSetup }),
  S("onboarding", "setup", "first-run-no-claude", ""),
  S("onboarding", "setup", "first-run-signed-out", ""),
  S("onboarding", "setup-name-missing", "first-run", "", { act: async (p) => { await fillSetup(p, ["Your name"]); await openOffice(p); } }),
  S("onboarding", "setup-recheck", "first-run-recheck", "", { act: (p) => p.getByRole("button", { name: "Check again" }).first().click() }),
  S("onboarding", "setup-opening", "first-run-opening", "", { act: async (p) => { await fillSetup(p); await openOffice(p); } }),
  S("onboarding", "setup-create-failed", "first-run-error", "", { act: async (p) => { await fillSetup(p); await openOffice(p); } }),
  S("onboarding", "handoff", "first-run", "", { act: async (p) => { await fillSetup(p); await openOffice(p); } }),

  S("org", "chart", "full", "#/org"),
  S("org", "chart-people", "full", "#/org", { act: (p) => p.getByRole("button", { name: "People", exact: true }).click() }),
  S("org", "chart", "empty", "#/org"),
  S("org", "chart", "loading", "#/org"),
  S("org", "chart", "error", "#/org"),
  S("org", "chart", "quiet", "#/org"),
  S("org", "chart", "large-org", "#/org"),
  S("org", "person-requests", "large-org", "#/org/kofi"),
  S("org", "panel-new-hire", "large-org", "#/org/wren/work"),
  S("org", "panel-work", "full", "#/org/otto/work"),
  S("org", "panel-work-attention", "full", "#/org/ines/work"),
  S("org", "panel-schedule-editor", "full", "#/org/mae/work", { act: (p) => p.locator(".sched-main").first().click() }),
  S("org", "panel-skills", "full", "#/org/juniper/skills"),
  S("org", "panel-skills-organized", "full", "#/org/otto/skills"),
  S("org", "panel-skill-file", "full", "#/org/juniper/skills/file~price-changes~SKILL.md"),
  S("org", "panel-skill-diff", "full", "#/org/juniper/skills/rev~rev-weekly"),
  S("org", "panel-profile", "full", "#/org/theo/profile"),
  S("org", "person", "full", "#/org/kofi"),
  S("org", "you", "full", "#/org/you"),
  S("org", "hire-sheet", "full", "#/org", { act: (p) => p.locator(".org-foot .btn-primary").first().click() }),
  S("org", "hire-sheet-template", "empty", "#/org", { act: (p) => p.locator(".org-empty-templates .chip", { hasText: "Chief of Staff" }).click() }),
  S("org", "add-person-sheet", "full", "#/org", { act: (p) => p.locator(".org-foot .btn:not(.btn-primary)").first().click() }),

  S("conversation", "thread-working", "full", "#/chat/juniper/thread~plan-1"),
  S("conversation", "thread-work-open", "full", "#/chat/juniper/thread~plan-1", { act: (p) => p.locator(".wl-line").last().click() }),
  S("conversation", "thread-menu", "full", "#/chat/otto/thread~slow-1", { act: (p) => p.getByRole("button", { name: "More" }).click() }),
  S("conversation", "thread-delegated", "full", "#/chat/otto/thread~billing-1"),
  S("conversation", "thread-queued", "full", "#/chat/rafi/thread~widget-2"),
  S("conversation", "thread-reply", "full", "#/chat/otto/thread~slow-1"),
  S("conversation", "thread-reader", "full", "#/chat/otto/thread~slow-1", { act: openNotes }),
  S("conversation", "thread-reader", "notes-gone", "#/chat/otto/thread~slow-1", { act: openNotes }),
  S("conversation", "thread-reader-text", "full", "#/chat/rafi/thread~flaky-1", { act: openNotes }),
  S("conversation", "thread-proposal-card", "full", "#/chat/mae/thread~refund-1"),
  S("conversation", "thread-report-card", "full", "#/chat/otto/thread~pg-1"),
  S("conversation", "thread-items-card", "full", "#/chat/rafi/thread~flaky-1"),
  S("conversation", "thread-two-cards", "full", "#/chat/theo/thread~news-1"),
  S("conversation", "thread-question-card", "full", "#/chat/mae/thread~triage-1"),
  S("conversation", "thread-chore-card", "full", "#/chat/theo/thread~stats-1"),
  S("conversation", "thread-task-card", "full", "#/chat/ines/thread~close-1"),
  S("conversation", "thread-blocked", "full", "#/chat/theo/thread~mail-1"),
  S("conversation", "thread-tidy", "full", "#/chat/juniper/thread~tidy-1"),
  S("conversation", "thread-failing", "full", "#/chat/ines/thread~recon-3"),
  S("conversation", "thread-snoozed", "full", "#/chat/juniper/thread~board-1"),
  S("conversation", "thread-archived", "full", "#/chat/mae/thread~welcome-1"),
  S("conversation", "new", "full", "#/chat/mae"),
  S("conversation", "thread", "loading", "#/chat/juniper/thread~plan-1"),
  S("conversation", "thread", "error", "#/chat/juniper/thread~plan-1"),
  S("conversation", "thread", "thread-error", "#/chat/juniper/thread~plan-1"),
  S("conversation", "thread-three-live", "live-more", "#/chat/juniper/thread~plan-1"),
  S("conversation", "thread-many-steps", "live-more", "#/chat/otto/thread~slow-1"),
  // Live work arriving (Mae is asked 3 s in) and all of it ending (by 8 s): shot once each has landed.
  S("conversation", "thread-live-arrived", "live-arrives", "#/chat/juniper/thread~plan-1", { act: (p) => p.waitForTimeout(4800) }),
  S("conversation", "thread-live-ended", "live-ends", "#/chat/juniper/thread~plan-1", { act: (p) => p.waitForTimeout(8800) }),

  S("inbox", "inbox", "full", "#/inbox"),
  S("inbox", "inbox-entry-open", "full", "#/inbox", { act: (p) => p.locator(".inbox-list .item-title").first().click() }),
  S("inbox", "card-asked", "full", "#/inbox", { act: askOnCard(false) }),
  S("inbox", "card-answered", "full", "#/inbox", { act: askOnCard(true) }),
  S("inbox", "inbox-keys", "full", "#/inbox", { act: (p) => p.getByRole("button", { name: "Keyboard shortcuts" }).click() }),
  S("inbox", "proposal-sheet", "full", "#/inbox", { act: (p) => p.getByRole("button", { name: "Review & hire" }).click() }),
  S("inbox", "snoozed", "full", "#/inbox/snoozed"),
  S("inbox", "parked", "full", "#/inbox/parked"),
  S("inbox", "archive", "full", "#/inbox/archive"),
  S("inbox", "inbox", "empty", "#/inbox"),
  S("inbox", "archive", "empty", "#/inbox/archive"),
  S("inbox", "inbox", "loading", "#/inbox"),
  S("inbox", "inbox", "error", "#/inbox"),
  S("inbox", "inbox", "inbox-more", "#/inbox"),

  S("activity", "runs", "full", "#/activity"),
  S("activity", "skill-changes", "full", "#/activity/skills"),
  S("activity", "runs", "empty", "#/activity"),
  S("activity", "runs", "loading", "#/activity"),
  S("activity", "runs", "error", "#/activity"),

  S("settings", "settings", "full", "#/settings"),
  S("settings", "settings-end", "full", "#/settings", { act: scrollToEnd }),
  S("settings", "settings", "loading", "#/settings"),
  S("settings", "settings", "error", "#/settings"),
  S("settings", "settings-middle", "full", "#/settings", { act: scrollTo("Replies from people") }),
  // The proxy is one row and Set up while it is off; Set up opens its fields in place.
  S("settings", "settings-proxy", "full", "#/settings", { act: async (p) => {
    await scrollTo("CLI proxy")(p);
    await p.getByRole("button", { name: "Set up", exact: true }).click();
    await p.getByLabel("CLI proxy address").click();
  } }),
  S("settings", "settings-saving", "slow-saves", "#/settings", { act: settingsChange("Pause schedules") }),
  S("settings", "settings-saved", "slow-saves", "#/settings", { act: settingsChange("Pause schedules", 1600) }),
  // A failure's line fades in after the save answers (0.6 s); the shot waits for it to settle.
  S("settings", "settings-save-failed", "failed-saves", "#/settings", { act: settingsChange("Pause schedules", 700) }),
  S("settings", "settings-background-failed", "failed-saves", "#/settings", { act: settingsChange("Run in the background", 700) }),
];

// ---------------------------------------------------------------- running

function fail(message) {
  console.error(`preview-shots: ${message}`);
  process.exit(1);
}

function options(argv) {
  const o = { out: join(tmpdir(), "omega-preview-shots"), only: "", url: "" };
  for (let i = 0; i < argv.length; i++) {
    const k = argv[i].replace(/^--/, "");
    if (!(k in o) || argv[i + 1] === undefined) fail(`unknown or empty option ${argv[i]}. Use --out <dir>, --only <text>, --url <dev server>.`);
    o[k] = argv[++i];
  }
  return o;
}

function playwright() {
  const require = createRequire(import.meta.url);
  const places = [process.env.PLAYWRIGHT_CORE, "playwright-core"].filter(Boolean);
  for (const place of places) {
    try { return require(place); } catch { /* next */ }
  }
  fail("playwright-core not found. Set PLAYWRIGHT_CORE to a playwright-core folder, or install it here without saving:\n  npm install --no-save playwright-core");
}

/** The dev server serves this folder: Vite only serves files under its own root through /@fs/. */
async function servesUs(url) {
  try { return (await fetch(`${url}/@fs${ROOT}/index.html`)).ok; } catch { return false; }
}
async function answers(url) {
  try { await fetch(url); return true; } catch { return false; }
}

async function devServer(url) {
  if (url) {
    if (!(await answers(url))) fail(`nothing answers at ${url}.`);
    return { url, stop: () => {} };
  }
  let port = 1420;
  if (await servesUs(`http://localhost:${port}`)) return { url: `http://localhost:${port}`, stop: () => {} };
  while (await answers(`http://localhost:${port}`)) port++;
  const vite = spawn(join(ROOT, "node_modules/.bin/vite"), ["--port", String(port), "--strictPort"], { cwd: ROOT, stdio: "ignore" });
  vite.on("error", () => fail("couldn't start Vite. Run npm install in this folder first."));
  const found = `http://localhost:${port}`;
  for (let i = 0; i < 120 && !(await servesUs(found)); i++) await new Promise((r) => setTimeout(r, 250));
  if (!(await servesUs(found))) fail(`Vite didn't come up on ${found}.`);
  console.log(`Started the dev server on ${found}`);
  return { url: found, stop: () => vite.kill() };
}

async function shoot(browser, base, shot, out) {
  const [width, height] = shot.size;
  const context = await browser.newContext({ ...BROWSER, viewport: { width, height }, deviceScaleFactor: 2, colorScheme: shot.theme });
  await context.clock.setFixedTime(CLOCK);
  const page = await context.newPage();
  const errors = [];
  page.on("pageerror", (e) => errors.push(e.message));
  // The app ships no favicon, so the browser's own request for one is not the page's error.
  page.on("console", (m) => { if (m.type() === "error" && !m.location().url.endsWith("/favicon.ico")) errors.push(m.text()); });
  try {
    await page.goto(base + shot.url, { waitUntil: "networkidle" });
    await page.evaluate(() => document.fonts.ready);
    await page.waitForTimeout(700);
    if (shot.act) {
      await shot.act(page);
      await page.waitForTimeout(700);
    }
    await page.screenshot({ path: join(out, shot.file) });
  } catch (e) {
    errors.push(`shot failed: ${e.message.split("\n")[0]}`);
  } finally {
    await context.close();
  }
  return errors;
}

const opts = options(process.argv.slice(2));
const { chromium } = playwright();
const shots = SHOTS.flatMap((s) => THEMES.flatMap((theme) => SIZES.map((size) => {
  const query = [`preview=${s.state}`, `theme=${theme}`, s.query].filter(Boolean).join("&");
  return { ...s, theme, size, url: `/?${query}${s.hash}`, file: `${s.area}--${s.screen}-${s.state}--${theme}--${size[0]}x${size[1]}.png` };
}))).filter((s) => s.file.includes(opts.only));
if (!shots.length) fail(`no shot's file name contains "${opts.only}".`);

const server = await devServer(opts.url.replace(/\/$/, ""));
let browser;
try {
  browser = await chromium.launch({ channel: "chrome" });
} catch (e) {
  server.stop();
  fail(`couldn't start Google Chrome (${e.message.split("\n")[0]}). Install it, on macOS at /Applications/Google Chrome.app.`);
}

// The first load makes Vite bundle the app's dependencies, and it reloads the page when it has; let that happen
// before anything is shot.
{
  const page = await browser.newPage();
  await page.goto(`${server.url}/?preview=full#/org`, { waitUntil: "networkidle" }).catch(() => {});
  await page.waitForTimeout(1500);
  await page.close();
}

await mkdir(opts.out, { recursive: true });
const manifest = [];
const queue = [...shots];
await Promise.all(Array.from({ length: WORKERS }, async () => {
  for (let shot = queue.shift(); shot; shot = queue.shift()) {
    const errors = await shoot(browser, server.url, shot, opts.out);
    manifest.push({ file: shot.file, area: shot.area, screen: shot.screen, state: shot.state, theme: shot.theme, width: shot.size[0], height: shot.size[1], url: `${server.url}${shot.url}`, ...(errors.length ? { errors } : {}) });
    console.log(`${errors.length ? "!" : " "} ${shot.file}${errors.length ? `  ${errors[0]}` : ""}`);
  }
}));
await browser.close();
server.stop();

// A run with --only replaces its own shots in the folder's manifest and keeps the rest.
const kept = await readFile(join(opts.out, "manifest.json"), "utf8").then((t) => JSON.parse(t).shots, () => []);
const all = [...kept.filter((k) => !manifest.some((m) => m.file === k.file)), ...manifest].sort((a, b) => a.file.localeCompare(b.file));
await writeFile(join(opts.out, "manifest.json"), `${JSON.stringify({ clock: CLOCK.toISOString(), ...BROWSER, shots: all }, null, 2)}\n`);
const bad = manifest.filter((m) => m.errors).length;
console.log(`\n${manifest.length} shots in ${opts.out}${bad ? `, ${bad} with errors (see manifest.json)` : ""}`);
