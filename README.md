<p align="center">
  <img src=".github/assets/banner.png" alt="Omega Star" width="80%">
</p>

<h3 align="center">Build your $1B one-person company on the Claude Code you've already set up.</h3>

<p align="center">A desktop app that turns your Claude Code into a team of AI employees. Give them roles, put them in an org chart next to the real people you work with, and teach them by talking.</p>

<p align="center">
  <a href="#quickstart"><strong>Quickstart</strong></a> &middot;
  <a href="#see-it-work">See it work</a> &middot;
  <a href="#how-it-works">How it works</a> &middot;
  <a href="#faq">FAQ</a>
</p>

<p align="center">
  <a href="LICENSE"><img alt="License: AGPL-3.0" src="https://img.shields.io/badge/license-AGPL--3.0-blue?style=flat-square"></a>
  <img alt="Platforms: macOS and Windows" src="https://img.shields.io/badge/platforms-macOS%20%7C%20Windows-lightgrey?style=flat-square">
  <img alt="Status: alpha" src="https://img.shields.io/badge/status-alpha-orange?style=flat-square">
  <img alt="Requires Claude Code" src="https://img.shields.io/badge/requires-Claude%20Code-555555?style=flat-square">
</p>

<p align="center">
  <sub>Proudly sponsored by <a href="https://vibetyper.com"><img src="https://vibetyper.com/images/vibe-typer-logo.png" alt="" width="16" height="16" align="absmiddle"> <strong>Vibe Typer</strong></a></sub>
</p>

**Keep the agent you trust. Omega Star adds the company around it.**

Omega Star is not another agent. Every AI employee (an autonomous Claude Code agent) is your own Claude Code, running headless (`claude -p`) under your login, with the CLI tools, logins and skills you've already set up. Omega Star adds what turns them into a team: roles, an org chart that includes real people, schedules, managers, and skills that grow as you teach. Use it to run your team, your project or your business.

## See it work

A week with Sam, your Chief of Staff, and Felix, a Cloud Engineer who reports to Sam:

```text
Monday 10 am, chat with Sam

  You    Every Monday I want one page: status line first, then evidence.
  Sam    Done. That's a rule in my weekly report skill now, and I've set
         a "Monday report" schedule for 8 am.
         [Skill refined: sam-weekly-report]

Next Monday 8 am, window closed, Omega Star in the tray

  Sam delegates the infrastructure review to Felix. Felix finds a
  certificate expiring Friday, sees on the org chart that Grace owns
  DNS, and asks her on Teams (through a local Microsoft 365 MCP server):

  Felix  Hi Grace, our API certificate expires Friday. Could you make
         the DNS change? Felix (AI assistant to Alex)

  Inbox  Waiting on people: Felix asked Grace · watching for a reply
  Inbox  Sam: Monday report. Status: on track, one open item (DNS).

9:15 am  Grace replies on Teams. Omega Star reads it and resumes Felix's work.
```

## Why Omega Star

You tried the agent frameworks. You went back to Claude Code. Omega Star starts where you ended up.

### Don't rebuild the agent

Personal agent frameworks try to reinvent the wheel. Each one builds its own agent loop, tool calling, browser and integrations, so setup is full of friction, results fall short, and you end up in integration hell. Many people drift back to doing their automations and daily work in a coding agent like Claude Code or Codex. [One Hacker News thread](https://news.ycombinator.com/item?id=47783940) is full of them, summed up as: "I could achieve everything it did with a long running Claude Code instance and a good mobile frontend."

There's a good reason. The top coding harnesses are simply the best at what agents actually do all day: calling tools, running commands and using a browser. So every Omega Star employee is your own Claude Code, not a reimplementation of it.

### Your computer is already set up

The environment is the other half. Your machine already has your apps, your CLI tools, your logins and your skills. An agent that runs there can use all of it from the first minute, with nothing to connect. That's what makes a personal agent system powerful, and it's what frameworks and cloud agents have to rebuild.

### Think in people, not prompts

Making an agent autonomous is hard mostly because it's hard to decide what it should do. Thinking of agents as people makes that easy. You hire a specialist for a job, give them an objective and the tools they need, and tell them who they report to.

The real people you work with go on the same org chart, each with a description, their strengths and how to reach them. Employees know when to bring in a human and how, just like a colleague would. If you'd hire someone for a job in your team, project or business, you can hire an employee for it.

### Teach them like a colleague

You teach an employee by talking to them, the way you'd teach a new hire. Each lesson becomes a skill, built in the small, composable style popularised by [Matt Pocock's skills](https://github.com/mattpocock/skills) on Anthropic's open [Agent Skills](https://agentskills.io) format. Employees organise and tidy their own skills as they grow, and every change can be undone.

## How it works

| Step | You | Your employee |
|---|---|---|
| **1. Hire** | Pick a template or start blank. Set a title, an objective and skills. | Gets a role skill that starts from that objective. |
| **2. Teach** | Explain a task in chat, or correct a report by replying to it. | Writes it into a skill and names the one that changed. |
| **3. Delegate** | Give them a schedule, a manager and people to ask. | Works on their own, asks when stuck, reports back. |

- **People.** Record each person's role, strengths and channels (Teams, Slack, email, Jira and more), each with a "When to use" line. Employees reach out with your Claude Code's own tools, and Omega Star tracks each request: a reply resumes the work, and without one the employee nudges once, then carries on.
- **Managers and colleagues.** Managers delegate to their direct reports, employees ask each other for help, and any of them can propose a hire for you to approve.
- **Several things at once.** While an employee is busy, anything else you send starts a task: a conversation of its own that runs alongside the rest of their work and knows what you'd just discussed. Archive it when it's done and their ongoing chat hears what was decided.
- **Schedules and the Inbox.** Schedules run from the tray and catch up once after the computer was off or asleep, unless you set them to skip. Reports, hire proposals, open requests and blocked actions land in one Inbox.
- **Skills that grow.** Each employee's role skill routes to small, one-job skills they write as you teach. Tidies run on their own, big restructures wait for your approval, a rule you taught is put back if a tidy drops it, and every skill edit can be undone.
- **You stay in charge.** New employees are Supervised: they edit files in their own folders, and anything else you haven't allowed is blocked until you click **Allow once & retry** or **Always allow & retry**. **Pause schedules** stops all scheduled work.
- **Runs on your Claude Code.** Every run, including the short Haiku calls that check for replies, is a `claude -p` process under your login. Omega Star has no agent loop or API client of its own, and nothing to host.

## Is it for you?

**A good fit** if you use Claude Code most days, have recurring work for a team, a project or a business, and would rather keep your setup than rebuild it.

**Not a fit** if you want a chat bot on your phone, other models (Omega Star runs Claude Code only), an agent that works while your computer is off or asleep, sandboxed agents, or a tool your whole team signs into (it's one person on one computer).

## Quickstart

**You need:** Claude Code installed and signed in (`claude auth status` shows `loggedIn: true`).

1. **Download and install** the app for your computer from the [latest release](https://github.com/itsfutsy/Omegastar/releases/latest):
   - **macOS:** the `.dmg` (`aarch64` for Apple Silicon, `x64` for Intel).
   - **Windows:** the `-setup.exe`. If SmartScreen appears, click **More info**, then **Run anyway**.
   - **Linux:** the `.AppImage`, `.deb` or `.rpm` (untested).
2. **Open your office.** Setup checks Claude Code and asks for your company, name and title. Turn on "Keep Omega Star running in the background" so it starts at login. Closing the window leaves it in the tray, where schedules keep running.
3. **Hire your first employee** from a template, such as Chief of Staff. Their chat opens with a first lesson already written. Send it, and they'll interview you and write their first skill.

**Tip:** start with one employee and one job, and expect a short training period while you allow the commands and tools they need.

<details>
<summary>Build it yourself</summary>

You need Rust (stable), Node.js 20.19+ or 22.13+, the Xcode Command Line Tools on macOS, and on Windows the MSVC C++ Build Tools with a Windows SDK.

```
git clone https://github.com/itsfutsy/Omegastar.git
cd Omegastar
npm install
npm run tauri dev
```

`npm run tauri build` makes installers for the platform you're on. On Windows, use PowerShell or cmd rather than Git Bash, or the build fails.

</details>

## FAQ

<details>
<summary><b>How is this different from OpenClaw or Hermes Agent?</b></summary>

They are personal agents with their own runtime, tools and chat-app gateways, built for many models, which you set up and maintain. Omega Star builds none of that: each employee is your own Claude Code. Hermes Agent also learns skills; here, what you teach lands in named skills you can diff and undo. For a model-agnostic assistant you chat with from your phone, they are the better fit.

</details>

<details>
<summary><b>Can't I do this with Claude Code, cron and a CLAUDE.md?</b></summary>

You can, and plenty of people do. Omega Star is that setup without the glue scripts: roles and an org chart that includes real people, schedules, one Inbox, one-click approvals, and skills that are versioned and kept tidy. It also handles the fiddly parts, like resuming the right session when a person finally replies, nudging once then moving on, and holding upkeep back when usage is high.

Claude Code's own sub-agents, agent teams and scheduled tasks are building blocks. Omega Star gives them a standing team: employees keep their role, skills, schedules and manager across restarts, and still use sub-agents inside a run.

</details>

<details>
<summary><b>Is it safe to give employees my computer?</b></summary>

Employees run as you, and there is no sandbox; containers isolate better. Supervised (the default) edits files only in the employee's folder and any folders you add, and blocks every other tool you haven't allowed, in Omega Star or in your own Claude Code settings. That includes your user-level MCP servers and claude.ai connectors, which every employee can reach. Browser control is off by default. Autonomous blocks nothing, and the app warns you.

</details>

<details>
<summary><b>What does it cost, and is it allowed on my plan?</b></summary>

Omega Star is free and open source; you pay for your Claude plan as usual. It runs the official Claude Code CLI as you and never reads your Claude credentials, so Anthropic's terms for your plan apply (see [Using it responsibly](#using-it-responsibly)). Work runs are capped at 3 at a time by default; chats, and work a manager delegates while it waits, skip the cap. The sidebar shows your 5-hour and 7-day Claude usage, and background upkeep (never real work) waits when usage is high.

</details>

<details>
<summary><b>Will an employee spam my colleagues or pretend to be me?</b></summary>

Employees are told to ask only when stuck, send one clear request and sign as your AI assistant, and each request gets one nudge at most. Those are instructions to the model; Omega Star itself sends nothing. The hard stop is Supervised: a messaging tool you haven't allowed stays blocked until you approve it.

</details>

<details>
<summary><b>Does it integrate with Slack, Jira, email or Teams?</b></summary>

It ships none. Employees use your CLI tools, user-level MCP servers, claude.ai connectors and the plugins you grant them, but not MCP servers set up for one project folder. When an employee asks on Teams through a local Microsoft 365 MCP server (`@softeria/ms-365-mcp-server`), Omega Star reads the reply itself. Elsewhere a short reply check looks for it, or you answer for the person.

</details>

<details>
<summary><b>Can I bring my own skills? Will it touch my setup?</b></summary>

Yes. Grant skills from your Claude Code catalog, or import copies for an employee to organise. Omega Star changes your skills folder only when you click **Make a company skill** or ask it to move imported originals to the Trash. Employees live in Omega Star's own folder, and there's no account, cloud service or telemetry.

</details>

<details>
<summary><b>Isn't the one-person company line just hype?</b></summary>

Fair. Nobody builds a company alone, which is why real people sit on the org chart. The headline is the ambition; everything under it is what the app does today.

</details>

## Using it responsibly

Omega Star runs your own Claude Code under your own login, so your use is covered by Anthropic's terms for your plan and its [Usage Policy](https://www.anthropic.com/legal/aup). In particular:

- Use your own account. Don't share a login or run Omega Star for other people on your subscription.
- You are responsible for what your employees do through their tools, including messages to real people. Employees are instructed to sign every message as an AI assistant; keep it that way.
- Pro and Max limits assume ordinary, individual use. For heavy or round-the-clock schedules, an API key or a Team or Enterprise plan is the better fit.

Omega Star is an independent project, not affiliated with or endorsed by Anthropic. Claude and Claude Code are trademarks of Anthropic.

## Development

Omega Star is a native desktop app: a Rust backend (Tauri 2) with a React frontend rendered in the operating system's webview.

```
npm run typecheck
npm run lint

cd src-tauri
cargo clippy --all-targets
```

Your office lives in the app data directory: `~/Library/Application Support/OmegaStar` on macOS, `%APPDATA%\OmegaStar` on Windows and `~/.local/share/OmegaStar` on Linux. Set `OMEGASTAR_DATA_DIR` to use another folder. Errors are logged to `omega.log` there.

Each employee has a folder there with a generated `CLAUDE.md` handbook, their own skills and a scratch workspace. A per-run `settings.json` hides skills they weren't granted, and office tools (delegation, requests, schedules) use an MCP endpoint the app serves on 127.0.0.1.

## License

Copyright (C) 2026 Abram Industries Pty Ltd

Omega Star is licensed under the [GNU Affero General Public License v3.0](LICENSE). Commercial licences
without the AGPL's terms are available from Abram Industries ([abramindustries.com](https://abramindustries.com)).
