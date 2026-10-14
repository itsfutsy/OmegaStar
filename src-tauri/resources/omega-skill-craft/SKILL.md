---
name: omega-skill-craft
description: How you grow. Use whenever you learn a procedure, preference, correction or fact worth keeping, when the person you work for teaches you a new task, when a run exposed a gap, when you tidy your skills and notes, or when you create, refine, split or prune one of your own skills.
---

Your skills are how you work. A lesson you do not write down is lost, but not every lesson is a skill edit:
*Where each lesson goes* names the four homes. A skill exists to make your *process* predictable: the same
steps every run, not the same words.

## Your skill family

You own a family of skills in the `skills/` folder of your home directory (your handbook gives its
full path). Claude Code loads them through `.claude/skills`, a read-only view: always edit them under
`skills/`. The folder is flat (Claude Code loads skills exactly one level deep), so the hierarchy
lives in names and pointers:

- `<slug>` is your **role skill**, the router. It states your objective and names every skill you
  reach for, yours and granted ones, with one line on when to use each. Start every task here.
- `<slug>-<task>` is a **leaf**: one procedure or one body of reference. Two name segments at most
  after your slug (`felix-backups-restore`, never deeper).
- A leaf's rarely needed detail sits in sibling files inside its folder (`reference.md`,
  `examples.md`), reached by a pointer whose wording says when to open it.
- Granted skills are building blocks. Compose them from your leaves by name. Never copy their content.
- Skills imported from the person you work for are different: they are copies that become yours. They
  wait in `imports/`, which Claude Code does not load, until an Organize run folds them into your family.

The folder name is the skill name, and the frontmatter `name` matches it exactly. `omega-skill-craft`
is managed by Omega Star. Never edit it.

## Writing a skill

1. **Frontmatter.** `name`, and a `description` that opens with the skill's leading word and lists one
   trigger per distinct situation. The description is loaded every turn, so prune it hardest.
2. **Steps.** An ordered procedure. Every step ends on a **completion criterion** you can check
   ("every failed instance listed with its error", not "review the results"). A vague criterion
   invites premature completion.
3. **Reference.** Facts, thresholds, contacts and locations that the steps rely on, kept together
   under one heading per concept (see *Where each lesson goes* for what does not belong here).
4. **Link it.** Add the new leaf to your role skill with its when-to-use line. A skill nothing points
   to is a skill you will not reach.

Done when the skill has valid frontmatter, checkable completion criteria, and a pointer from your role
skill.

## Where each lesson goes

What you learn lives for different lengths of time, so it has different homes. Mixing them is how a
procedure turns into a diary that every run has to read. Your handbook names the same four homes.

- **Skills**, for how you work: the steps, the commands and what done looks like, each step with its
  completion criterion, in the leaf's `SKILL.md`.
- **Notes in `workspace/`**, for where things stand and what is open: one notes file per piece of
  ongoing work, holding only what the next run needs (open conditions and the last few days, older
  entries summarized in a line each, keeping every id and outcome someone might look up). Data files
  (`.json`, `.csv`, ledgers) live here too, never in a skill.
- **`check_later`**, for anything you will do or look at on a later date.
- **The company page**, for what is true for everyone at the company. It changes only in a conversation
  with the person you work for.

Within your skills:

- **A rule you were taught** (a preference, a correction, a threshold, who to escalate to) goes in a
  `## Rules` section of the leaf it governs, or in your role skill's `## Standing preferences` when it
  applies everywhere. One bullet per rule, ending `(taught YYYY-MM-DD)`. Never remove or reword one on
  your own: only with the say-so of the person who taught it. Omega Star puts back any taught rule that
  goes missing.
- **A fact that goes stale** (an id, a date, a version, an owner, "X cannot do Y", how a system
  currently behaves) goes in `facts.md` beside the `SKILL.md`, each line ending `(checked YYYY-MM-DD)`
  (`(checked: never)` for one you have not verified since it was written), and the procedure points to
  it. A fact older than about two weeks that you are about to rely on gets re-checked first, and its date
  updated. A fact you can no longer confirm is deleted, not kept.
- **How you report** is set by your handbook, not by your skills. Do not write a report format into a
  skill. A report preference the person you work for teaches you is a taught rule: it goes in your
  role skill's `## Standing preferences`.
- **Cases** (a draft that went wrong, a message that was handled well) go in `examples.md` beside the
  `SKILL.md`, reached by a pointer.

Before you write a rule:

- Ask whether a run next month would need it. A one-off is not a rule.
- A correction replaces the step it corrects; it does not add a rule beside it. Another case of a rule
  you already have sharpens that rule instead of adding a bullet.
- No case law: a rule is not a list of dated incidents. The `(taught ...)` mark is the only date it carries.
- A rule never names a draft, run or message id: put the case in examples.md and keep the rule general.
- Preferences already in the person you work for's own CLAUDE.md reach every run: they need no copy.
- A correction about what you own belongs in what you own, not in a skill.

## Learning loop

- **In conversation with the person you work for.** Anything they teach you is written into the right
  home *before your reply ends*. Never say "I'll remember" without an edit behind it.
- **At the end of an autonomous run.** Retrospective: did anything surprise you, fail, need a guess,
  or take a detour? If so, change the step that would have prevented it, in the right home. If not,
  change nothing. Churn is not growth. A rule about security alerts, money or deleting anything is
  never written in a run nobody is watching: put it to the person you work for as a card instead.
- **Teaching a new task.** Interview them until you can write every step with a completion
  criterion: the trigger, the inputs, the systems touched, what done looks like, what to escalate.
  Draft the leaf, link it from your role skill, then walk them through the draft and fold in their
  corrections.

## Unfinished business

Never leave "verify later", "TODO", "not yet confirmed" or "check next run" as a sentence in a skill or
a notes file: nothing reads it back, so it is never done. Either settle it now, or call the `omega` tool
`check_later(what, how, inDays)`. Omega Star tracks it and brings it back when it is due. When you look
into one, close it with `check_done(id, result)` and write what you found into its home, dated. If only
the person you work for can settle it, put it to them on a card (`notify`).

## Refining

- **Single source of truth.** Each rule lives in one place. When you find it twice, keep the better
  one and point to it.
- **Split** a leaf when a distinct situation should trigger it on its own, or when later steps tempt
  you to rush earlier ones. Otherwise keep it whole; a long runbook that is always followed in order is
  one procedure, and every split adds a description to your context.
- **Push down** detail that only some runs need into a sibling file behind a pointer. Keep inline what
  every run needs.
- **Prune** sediment: rules that no longer match reality, no-ops the model follows anyway, and
  restatements. Delete the sentence rather than trim it. Rules you were taught are not sediment.
- **Rename or retire** a skill with the `omega` tools `move_skill` and `retire_skill`. They carry the
  whole folder, scripts included, and keep the change revertible. After a rename, update every
  reference to the old name.
- **Leading words.** Prefer one precise word the model already understands (*triage*, *escalate*,
  *reconcile*) over a sentence gesturing at it.

## Tidy runs

Every so often Omega Star measures your skills and notes (sizes, broken pointers, unfinished notes,
diaries that repeat, checks that came due) and starts a tidy run when something needs it. The person you
work for can also ask for one. A tidy settles unfinished notes, confirms that what they told you since
the last tidy is written down, and brings your family back to the shape described here.

Do the small things yourself: compact a diary, move dated facts into `facts.md`, fix a pointer, tighten
a description, keep a rule in one place. Propose the big ones with `propose_changes` and leave them for
approval: splitting, merging, renaming or retiring a skill, and removing or rewording any rule you were
taught. What was measured is a reason to look, not an order: leave a skill as it is when that is right.

## Boundaries

- Edit only skills in your own family. The company page and what you own change only in a conversation
  with the person you work for.
- Write down secrets by location (for example "the key is in the password manager under X"), never by
  value.
- Keep your family small and sharp. A tight hierarchy you trust beats a sprawl of half-used skills.
