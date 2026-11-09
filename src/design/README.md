# Design foundation

Direction C, Ultramarine: an ultramarine sidebar that is the team's home, black and white work surfaces, coral for
anything that needs you, heavy Inter Display on a strict grid. Every screen builds on the files here and in
`src/components/ui.*`; nothing in an area should need its own button, pill, menu or loading pattern.

See every part at once: `npm run dev`, then `http://localhost:1420/?preview=parts` (add `&theme=dark`).

## Tokens (`tokens.css`)

Light, dark (`[data-theme="dark"]` and the identical `prefers-color-scheme` block; keep them in step) and reduced motion.
Employee pages (the `show` tool) get this file too: `src-tauri` embeds it, so `--ink --ink-2 --muted --faint --canvas
--surface --raised --sunken --line --line-strong --live --ok --warn --err` (with each `-soft`) `--font-ui --font-mono
--r-card --r-inner` must stay defined, with the same meaning, in both themes.

| Group | Tokens | For |
|---|---|---|
| Sidebar | `--side` `--side-ink` `--side-muted` `--side-line` `--side-fill` `--side-fill-muted` `--side-fill-attn` `--side-on` `--side-on-ink` `--side-attn` `--side-focus` | Anything on ultramarine. Never put `--ink` or `--muted` text on `--side`. `--side-on` is the open page; `--side-fill` is fields, hover and an open member, and quiet text on it is `--side-fill-muted`; `--side-attn` is "needs you" in words (`--side-fill-attn` on the fill). |
| Surfaces | `--bg` `--canvas` `--surface` `--raised` `--sunken` `--grid-dot` `--grid-dot-strong` | `--canvas` is the page, `--raised` a column or foot, `--sunken` a well, hover or skeleton. |
| Ink | `--ink` `--ink-2` `--muted` `--faint` | `--muted` is the quiet text (6:1). `--faint` is for large text and marks only (3.4:1 in light, 3.7:1 in dark), never small words. Dark `--faint` is lifted from C's #596070 to #646b7b so it holds 3:1 on every dark surface. |
| Lines | `--line` `--line-strong` `--line-card` | Hairlines between rows; `--line-strong` for controls and cards. |
| Action | `--accent` `--accent-ink` `--accent-hover` `--accent-text` `--accent-soft` `--accent-line` | The one primary action per view: black in light, white in dark. |
| Selection | `--sel` `--sel-ink` `--sel-muted` `--sel-live` `--sel-attn` `--me` `--me-ink` | A selected row, option or chip; tooltips and toasts. Inverted in light, a raised step in dark. `--me` is your own message. |
| State | `--live` `--live-soft` `--live-text`, `--ok` `--ok-text` `--ok-soft`, `--warn` `--warn-soft`, `--err` `--err-text` `--err-soft` `--err-fill` | `--live` (ultramarine) means working, nothing else. Coral: `--err` for dots, bars and edges; `--err-text` for words; `--err-fill` behind white text. Green only for done. |
| Elevation | `--shadow-card` (none) `--shadow-float` `--float` `--scrim` `--focus` | Cards are 1px rules, never lifted. Only menus, the palette, sheets, toasts and tooltips float. `--float` is a popover's own surface: the page's in light, one step lighter in dark, with the float shadow's 1px edge. |
| Type | `--font-ui` `--font-mono` `--font-features` `--display`; `--t-xs` 11 `--t-sm` 12 `--t-base` 13.5 `--t-md` 15 `--t-lg` 22 `--t-xl` 30 `--t-title` 26 `--t-hero` 34; `--lh-tight` `--lh-body` | `--display` is Inter Display for names and titles, at weight 700 and -0.03 to -0.04em. 11px is the floor. Mono only for paths, commands and identifiers. |
| Space | `--sp-1` 4 to `--sp-8` 48 | An 8px grid with 4px halves. Words keep 12px from any edge. |
| Radii | `--r-card` 6 `--r-inner` 4 `--r-input` 5 `--r-pill` | Pills only for avatars, filter chips and status pills. |
| Motion | `--ease` `--ease-move` `--ease-arrive`; `--d-ui` 120 `--d-panel` 260 `--d-fold` 420 `--d-arrive` 320 `--d-settle` 380 `--d-swap` 200 `--d-roll` 240 `--d-fade` 140; `--stagger` 40 | See Motion below. |
| Layout | `--side-w` 216 `--rail-w` 60 `--outline-w` 288 `--work-w` 336 `--head-h` 64 `--chrome-top` | `--chrome-top` is the room the window's own controls take at the top of the sidebar: 30px on macOS (overlay title bar, where `--rail-w` is also 84 so the traffic lights clear it), 0 elsewhere. |

## Shared parts

| Part | Classes or component |
|---|---|
| Buttons | `.btn` with `.btn-primary`, `.btn-ghost`, `.btn-danger` (asks first), `.btn-danger-solid` (the confirm), `.btn-sm` (28px), `.btn-lg`, `.btn-icon`; a key inside: `<span class="kc">L</span>` |
| Inputs | `.input` `.textarea` `.select` `.input-affix`; `.field` `.field-label` (`.hint`) `.field-help` `.field-error`; `aria-invalid` for the error ring |
| Choosing | `.seg` (buttons with `aria-pressed`), `Tabs`, `Switch`, `.chip` (round filter, `aria-pressed`, a count in `.n`) |
| Labels | `.tag` (square kind: `.tag-attn` `.tag-live` `.tag-warn` `.tag-hire`), `.pill` (`.pill-live` `.pill-wait` `.pill-ok` `.pill-warn` `.pill-err`), `.kbd` |
| Status | A word first, colour saying the same: `StatePill` (tones `accent` working, `wait` up next, `ok`, `warn`, `err`, `neutral`), `.status` (`.status-live` `.status-wait` `.status-attn` `.status-fail` `.status-ok` `.status-plain`), `.dot` (`.dot-live` `.dot-wait` `.dot-attn` `.dot-ok`). Nothing pulses. `runStateLabel` and `runStateTone` (`src/lib/format.ts`) give a run's word and tone; queued is "Up next" on `wait`. |
| Avatars | `Avatar` with `person` (ringed), `proposed` (dashed), `you` (ink disc) and `presence` (a still dot ringed in `--av-ring`) |
| Menus and tips | `Popover`, `MenuButton`, `.menu` `.menu-item` (`.menu-item-danger`) `.menu-sep`. A tooltip: put `data-tip="..."` on anything; `TipLayer` (mounted once in `App`) draws it to the right, or the left near the window's edge, above every layer, so no scrolling sidebar or panel clips it, and follows the text if it changes while shown. `data-tip-key="K"` adds the key that does the same, as a keycap. A `.tip-words` child holding the same words keeps the tip quiet until a narrow layout hides them. |
| Sheets | `SlideOver` (flush right, full height; `modal` adds the scrim; `returnFocus` is where focus goes on close); `.sheet-head` `.sheet-body` `.sheet-form` `.sheet-foot`. `useShell()`'s `hire`, `reviewProposal` and `addPerson` take the opening button last (`from`), because WebKit does not focus a button on click; the hire sheet sends focus back to it, and ⌘ or Ctrl+Enter hires or approves. |
| Feedback | `toast(text, tone, action?, { ms })` returns its id for `dismissToast(id)`: an Undo toast passes its window as `ms`; a failure (`err`) stays until dismissed and shows a close; anything else leaves after 4.2 s. A page with a bar at its foot sets `--toasts-bottom` so toasts land above it (the Inbox does). `ErrorNote` (inline, or with `title` for what happened, what happens next and one way to act), `ConfirmButton` |
| Empty and loading | `.empty` `.empty-title` `.empty-actions`; `Skeleton` (still blocks at the content's own geometry) |
| Page frame | `PageHeader` (64px, 24px Display title), `PageColumn` (its header ends where its 820px column does, and the column fades under it as it scrolls), `Section` (its count rolls), `.plist` |
| Type | `.display` `.h-xl` `.h-lg` `.h-md` `.mono` `.num` `.trunc` `.muted`; `shortAge()` in `src/lib/format.ts` for list ages ("4m", "3h", "2d") |
| Icons | `Icon` (`src/components/Icon.tsx`), one 1.5 stroke on a 16px box. New: `chevronUp`, `minus`, `lines`. No sparkles: `sparkle` now draws `lines` and stays only until the areas move off it (`Transcript`, `TidyProposal`, `RunRow`, `CardView`); use `teach` for skills grown, `lines` for a summary, `checklist` or `layers` for a tidy. |
| Shell rules | `useRail()` and `railNow()` (`src/lib/sidebar.ts`): the rail below 1180px unless expanded there, each size keeping its own choice (onboarding's band uses the same). `macChrome`, `dragRegion` and `data-chrome="mac"` (`src/lib/chrome.ts`): the macOS overlay title bar's drag band. The overlay is set in `src-tauri/tauri.macos.conf.json`, which Tauri merges only into macOS builds; its merge replaces the `windows` array whole, so it repeats every field of the main window: change one, change both. `markPending` and `clearPending` (`src/lib/pending.ts`): an answer inside its Undo window, which the sidebar's Inbox badge leaves out. `openInbox(view, item)` and the `#/inbox/<view>/<item>` route, and `#/activity/numbers` (`src/lib/router.ts`). `setTheme` crossfades the window (a view transition) unless motion is reduced. `Shell`'s `arriving` drops the sidebar's parts into the band at the onboarding hand-off. `SideBrand`, `COLLAPSE_KEY`, `.side-top` and `.side-you*` keep their geometry; the sidebar's top grows by `--chrome-top` on macOS. |

## Motion

Nothing moves unless state changes: no pulses, breathing dots, shimmer or idle loops. A live dot holds still; a spinner
turns only while its action runs. Every move has a cause, moves things rather than swapping them, and has a reduced
version (a crossfade over `--d-fade`; nothing travels).

| Move | Use |
|---|---|
| `.enter-rise` / `.leave-rise` | One body giving way to the next: out with a short rise, in from 8px below (`--d-swap`) |
| `.enter-drop` | Something new arriving whole from just above: a card, a toast, a pill (`--d-arrive`, `--ease-arrive`) |
| `.enter-pop`, `.enter-fade`, `.leave-fade` | A menu or popover opening from its anchor; plain fades |
| `.collapse` + `data-open` | A row opening or closing its own space, so what is below moves with it |
| `Roll` (`ui.tsx`) | A count, an amount or a word rolling to its new value; down when the number goes down (numeric strings too). Numbers and amounts roll only the digits that changed, last digit first; keep a currency sign outside if it must not move. `render` draws each value, so an icon and its word roll as one (`CopyCommand`) |
| `useFlip` (`src/lib/motion.ts`) | Rows that move rather than swap when a list reorders or grows |
| `usePresence` (`src/lib/motion.ts`) | Keep something mounted while its leaving class plays |
| `.scroll-fade` + `useScrollFade` (`src/lib/scrollFade.ts`) | Fade only the edge of a scrolling area that hides something. `--band` keeps a solid strip under a header before the fade starts, and `--fade-top` sets that fade's height (`PageColumn` uses 12px for both) |

`pulseDot`, `breathe` and `shimmer` are gone, so the loops that used them are already still. Areas: delete the
leftover references (`OrgChart`, `Transcript`, `ChatTab`, `Panel`, `Conversation`), and switch coral words from
`color: var(--err)` to `--err-text` (`Activity`, `Inbox`, `Settings`, `Transcript`,
`Schedules`, `SkillsTab`). `.copycmd` now lives in `src/components/CopyCommand.css`; the Settings agent deletes its
copy from `Settings.css`.

## Decisions the owner settled

- Blocked actions use one pair of buttons everywhere: **Allow once** and **Always allow**; either one retries the run.
- **Up next** everywhere for work waiting to start: run rows, pills, the sidebar, the palette and the rail's
  tooltips. It is the hollow ring and a quiet word (`StatePill tone="wait"`), never ultramarine.
- A selected row is the **inverted grey selection** (`--sel`, `--sel-ink`, `--sel-muted`): ultramarine means working,
  only. No selection tint.
- The Inbox's words for what an item needs: Hire, Decide, Approve, Do, Answer, Know, Failed and **Reply** (`.tag`).

## Who owns what

The foundation (this folder, `src/components/ui.*`, `Shell.tsx`, `App.*`, `main.tsx`, `src/lib/` except
`orgLayout.ts`, `src/preview/`, `scripts/preview-shots.mjs`, `HireSheet`, `CommandPalette`, `UpdateCard`,
`UsageCard`, `CopyCommand`, `Icon`, `Sheet.css`) is frozen: ask the orchestrator for a change. Moved from it or from
another area:

- **Conversation** takes `BlockedCard`, `TidyProposal` and the card styles now in `Inbox.css` (the `card-*` rules and
  what its thread uses), moving them into its own stylesheet; the Inbox deletes them from `Inbox.css`.
- **Org** takes `src/lib/orgLayout.ts`. The member panel is now the person's page, so the panel's resize edge
  (`PanelEdge`, `src/lib/panelWidth.ts`, `--panel-w`) is gone; every `SlideOver` is a modal sheet.

## Preview and screenshots

The browser preview (`src/preview/README.md`) shows every screen with an invented office: `npm run dev`, then
`http://localhost:1420/?preview=<state>&theme=<light|dark>#<route>`. It never reaches a build.
`node scripts/preview-shots.mjs [--out <dir>] [--only <text>]` shoots every screen in light and dark at 980 x 640 and
1600 x 1000, with names that pair from run to run.
