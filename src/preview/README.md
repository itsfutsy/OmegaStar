# Browser preview

Every screen of Omega Star in a plain browser, without Tauri, so it can be seen and screenshotted. Dev only.

`npm run dev`, then open `http://localhost:1420/`. Outside Tauri, `src/api/client.ts` loads `transport.ts` here instead
of the desktop bridge. That branch sits behind `import.meta.env.DEV`, so `npm run build` drops it, and inside Tauri
(`npm run tauri dev` or a real build) nothing changes.

The office is invented: Fernbrook, a small company that makes booking software for fitness studios, run by Maren Holt
with six AI employees and three real people. Every name, figure and address is made up. Times are relative to when
the page loads.

## The URL

```
http://localhost:1420/?preview=<state>&theme=<theme>[&sidebar=collapsed][&toasts=1][&chrome=mac]#<route>
```

`?preview=parts` is not an office: it is every shared part on one page, from the real components (`Parts.tsx`; add
`&theme=dark`).

| Parameter | Values | Default |
|---|---|---|
| `preview` | a state below | `full` |
| `theme` | `light`, `dark`, `system` (saved, like the app's own setting) | unchanged |
| `sidebar` | `collapsed` (the rail at every size), `expanded` (the full sidebar at every size); saved | the app's rule: the rail below 1180 px |
| `toasts` | `1`: three toasts show once the office loads (the two that report something done leave after 4 seconds; the failure stays until dismissed) | off |
| `chrome` | `mac`: the macOS overlay title bar's room at the top of the sidebar (no traffic lights are drawn) | the browser's |

### States

| State | What it shows |
|---|---|
| `full` | A busy office: two employees working (one streaming), one waiting to start, two needing a look, a full Inbox, an update available |
| `empty` | A company with nobody hired yet |
| `first-run` | No company: onboarding, with Claude Code found and signed in |
| `first-run-no-claude` | Onboarding, Claude Code not found |
| `first-run-signed-out` | Onboarding, Claude Code found but signed out |
| `loading` | The office opens, then nothing else ever answers: every screen's loading state |
| `loading-boot` | Nothing answers: the startup screen |
| `error` | The office opens, then every call fails: every screen's error state |
| `error-boot` | The office can't open |
| `update-failed` | `full`, and Restart to update fails |
| `usage-limit` | `full` at the 5-hour limit: work waits for the reset |
| `api-key` | `full` on an API-key account: the sidebar counts runs instead of usage |
| `first-run-recheck` | Onboarding with Claude Code not found; Check again (from half a second after load) finds it, signed in |
| `first-run-opening` | Onboarding; Open your office never answers ("Opening…") |
| `first-run-error` | Onboarding; Open your office fails, and so does the start-at-login switch |
| `slow-saves` | `full`; every Settings save (`save_settings`, `set_background`, `backup_now`) answers after 1.5 seconds |
| `failed-saves` | `full`; those saves fail: "Couldn't save: the office folder is read-only." |
| `slow-sends` | `full`; every message you send from a conversation (`reply_to_run`, `start_task`) answers after 2.5 seconds, so it shows on its way |
| `failed-sends` | `full`; the first message you send fails ("Omega Star couldn't reach the office…") and stays with Retry; the next goes through |
| `quiet` | `full` with everything finished: nobody working, waiting or failing |
| `large-org` | `full` plus 14 employees and 2 people (25 members): `wren` (Data Analyst) was hired today and has no runs; Kofi has two open requests |
| `inbox-more` | `full` plus Inbox cases: three runs failing on one error (`smoke-1`, `links-1`, `widget-1`), Theo's two cards about the launch (`launch-1`, `launch-2`), Mae's question asked by Ines with other choices (`annual-1`), a three-line title (`dns-1`), a blocked MCP tool (`issues-1`) |
| `live-more` | `full` with three runs going on Juniper's thread (her reply, Otto's `billing-1` and Mae's `help-1`, both delegated); `billing-1` finishes 6 seconds in; `slow-1` has 22 steps |
| `thread-error` | `full`, but every conversation fails to load (`thread` errors) |
| `live-arrives` | `full`; 3 seconds in, Juniper asks Mae to update the help centre (`help-1`, delegated from `plan-2`), and Mae starts on it: live work arriving |
| `live-ends` | `full`; everything live finishes in turn (the sub-agent at 2.5 s, `billing-1` at 4 s, `plan-2` at 6.5 s, the runs waiting to start by 8 s), so nothing is live after that |

### Routes

The app's own hash routes (`src/lib/router.ts`), with these ids:

- Employees: `juniper` (Chief of Staff), `otto` (Cloud Engineer, reports to Juniper), `rafi` (QA, reports to Otto),
  `mae` (Support Lead), `theo` (Growth Marketer), `ines` (Bookkeeper). People: `harriet`, `kofi`, `lena`. You: `you`.
- Panels: `#/org/<id>` with `/work`, `/skills` or `/profile`. Skills files and changes:
  `#/org/juniper/skills/file~price-changes~SKILL.md`, `#/org/juniper/skills/rev~rev-weekly`.
- Conversations: `#/chat/<employee>/thread~<root run>`, or `#/chat/<employee>` for a new one. A run's
  old link, `#/run/<run>`, opens its conversation.
- Activity: `#/activity` (runs), `#/activity/numbers`, `#/activity/skills`.
- Analytics: `#/analytics`.
- The Inbox: `#/inbox/<view>/<item>`, view `inbox`, `snoozed`, `parked` or `archive`, item the id the Inbox gives an
  entry (`openInbox` in `src/lib/router.ts`). It opens that item once the Inbox reads the route.

| Thread or run | What it is |
|---|---|
| `plan-1` (Juniper) | Conversation; its reply `plan-2` streams live with tools, a hand-off to Otto, a sub-agent and a page |
| `billing-1` (Otto) | The hand-off, running |
| `slow-1` (Otto) | Conversation he answered: a reply waiting on you |
| `refund-1` (Mae) | Conversation with a card proposing a company-page line |
| `pg-1` (Otto) | Report with a choice card and a link |
| `flaky-1` (Rafi) | Report with a list card |
| `news-1` (Theo) | Report with two cards, a draft file and images |
| `triage-1`, `summary-1` (Mae) | A question card; news |
| `stats-1` (Theo), `close-1` (Ines) | A chore (sign in); a task only you can do |
| `mail-1` (Theo) | Blocked on a tool |
| `tidy-1` (Juniper) | A tidy proposing bigger changes |
| `recon-3` (Ines) | A routine failing three times in a row |
| `board-1` (Juniper) | Snoozed until tomorrow |
| `welcome-1` (Mae) | Archived |
| `widget-2` (Rafi) | Waiting to start |

Sheets, the command menu and Inbox tabs open the way they do in the app (a click, or Ctrl+K).

What you do mostly works: replies, new conversations and Run now answer with a short streamed reply, cards are
answered, threads archive, approvals and settings save. It all resets on reload.

## Screenshots

```
node scripts/preview-shots.mjs [--out <dir>] [--only <text>] [--url <dev server>]
```

Every screen in the list at the top of the script, in light and dark, at 980 x 640 and 1600 x 1000, at 2x, named
`<area>--<screen>-<state>--<theme>--<w>x<h>.png`, with a `manifest.json` giving each shot's URL. `--only` reshoots the
shots whose file name contains the text and updates just those in the manifest. The browser clock is held at
Thursday 5 November 2026, 2:30 pm Sydney time (en-AU), so two runs pair up as before and after.

Beyond one shot per screen and state, the list covers onboarding pressed with a name
missing, the re-check passing, "Opening…", the create failing and the hand-off (its fields are found by label);
Settings' middle sections, the proxy, and a save in progress, saved and failed; the quiet, large, Inbox and live-work
states; the sidebar expanded at every size and with the macOS title bar's room; the update's confirm and the rail's
armed button; the palette finding nothing.

It reuses a dev server already serving this folder on port 1420, or starts one. It needs Google Chrome and
`playwright-core`: set `PLAYWRIGHT_CORE` to a `playwright-core` folder, or install it here with
`npm install --no-save playwright-core`. Nothing is added to this project's dependencies.
