import { useCallback, useEffect, useRef, useState } from "react";
import { call, errorMessage } from "../../api/client";
import type { EmployeeView, Run, Thread } from "../../api/types";
import { askComposer } from "../../lib/composer";
import { firstName, threadTitle, whenInline, whenShort } from "../../lib/format";
import { useOrg, useQuery } from "../../lib/hooks";
import { setMark } from "../../lib/marks";
import { openChat, openMember } from "../../lib/router";
import { snoozePresets, threadStatus, type Status } from "../../lib/status";
import { toast } from "../../lib/toasts";
import { didNotFinish, doneHeld, isLive, rowThreads, startedByYou, threadFocus } from "../../lib/threads";
import { kindIcon, kindNames } from "../RunRow";
import { Icon, type IconName } from "../Icon";
import { Popover } from "../ui";
import { newConversationDraft, TEACH_PREFIX } from "./NewConversation";
import { markDone, reopen, WHERE_ARE_WE } from "./ThreadView";
import { employeeItems, useStatuses, type Item } from "./useStatuses";

/** Where a conversation stands, for its header and its actions: its runs, its state, and whether it can be archived. */
type ThreadState = {
  root: Run | null; runs: Run[]; title: string; status: Status | null; archived: boolean; task: boolean;
  /** It finished and nothing is running, so Archive can file it. */
  ready: boolean;
  /** A card waits on you: it is answered on the card, so neither Archive nor E closes it. */
  carded: boolean;
  /** The Inbox row it is in, when it leads one. */
  row: Item | null;
  failed: boolean;
};

/** The thread with `rootId` as its header and actions need it: read on its own (light, no transcripts), so the switcher
 *  works wherever it is placed. */
export function useThreadState(rootId: string | null): ThreadState {
  const { snap } = useOrg();
  const { items, filed, ctx } = useStatuses();
  const q = useQuery<Thread | null>(() => (rootId ? call("thread", { runId: rootId }) : Promise.resolve(null)), [rootId], ["run:updated"]);
  const t = q.data ?? null;
  const root = t?.root ?? null;
  const runs = t ? [t.root, ...t.replies] : [];
  const inRow = rootId ? items?.find((i) => rowThreads(i.thread).some((x) => x.root.id === rootId)) ?? null : null;
  const row = inRow?.thread.root.id === rootId ? inRow : null;
  const own = root ? { root, replies: runs.slice(1), related: [] } : null;
  const status = own ? threadStatus(own, !!(rootId && filed?.has(rootId)), ctx) : row?.status ?? null;
  const tail = runs[runs.length - 1];
  return {
    root, runs, status, row,
    title: root ? threadTitle(root, kindNames(root, snap?.employees ?? [], snap?.people ?? [])) : row?.title ?? "",
    archived: !!root?.archivedAt,
    task: !!root && startedByYou(root),
    ready: !!root?.endedAt && !root.archivedAt && !runs.some(isLive),
    carded: own ? doneHeld(own, inRow?.thread ?? own) : false,
    failed: !!tail && didNotFinish(tail) && !runs.some(isLive),
  };
}

/** Leaving the thread after you snooze it or archive it: call before acting, to fix the employee's next thread while this
 *  one is still among the items, then call what it returns to open it (a new conversation when none is left). */
export function useLeave(employeeId: string, rootId: string): () => () => void {
  const { items } = useStatuses();
  const latest = useRef(items);
  latest.current = items;
  return useCallback(() => {
    const mine = employeeItems(latest.current ?? [], employeeId);
    const i = mine.findIndex((x) => x.thread.root.id === rootId);
    const next = i < 0 ? null : mine[i + 1] ?? mine[i - 1] ?? null;
    return () => openChat(employeeId, next ? threadFocus(next.thread.root.id) : null, true);
  }, [employeeId, rootId]);
}

/** Archive the thread with `rootId` and move to the employee's next one. */
export function useArchive(employeeId: string, rootId: string) {
  const leave = useLeave(employeeId, rootId);
  return useCallback(async () => {
    const go = leave();
    if (await markDone(rootId)) go();
  }, [leave, rootId]);
}

/** Where a conversation stands when its talk does not say it: put away until later, or archived. Nothing otherwise:
 *  working, up next and needing you are said once, in the reply itself. */
export function QuietState({ s }: { s: ThreadState }) {
  if (s.status?.state === "snoozed") return <span className="convo-state">Later, until {whenInline(s.status.snoozedUntil)}</span>;
  if (s.archived) return <span className="convo-state">Archived</span>;
  return null;
}

/** A dot for where a conversation stands, as the menu lists it. */
const WORD: Partial<Record<Status["state"], string>> = { needsYou: "Needs you", working: "Working", queued: "Up next", snoozed: "Later" };

/**
 * The conversation's title, which opens a menu of the employee's other conversations (each with where it stands) and
 * New conversation. `rootId` is the open conversation, or null for a new one.
 */
export function ConversationPicker({ e, rootId, title }: { e: EmployeeView; rootId: string | null; title: string }) {
  const { items } = useStatuses();
  const anchor = useRef<HTMLButtonElement>(null);
  const [open, setOpen] = useState(false);
  const focus = rootId ? threadFocus(rootId) : null;
  const list: { focus: string; title: string; icon: ReturnType<typeof kindIcon>; status: Status | null }[] = employeeItems(items ?? [], e.id)
    .map((i) => ({ focus: threadFocus(i.thread.root.id), title: i.title, icon: kindIcon(i.thread.root), status: i.status }));
  if (focus && !list.some((x) => x.focus === focus)) list.unshift({ focus, title, icon: "chat", status: null });
  const go = (f: string | null) => { setOpen(false); openChat(e.id, f); };
  return (
    <>
      <h1 className="convo-title">
        <button ref={anchor} type="button" className="convo-pick" aria-haspopup="menu" aria-expanded={open} onClick={() => setOpen((o) => !o)} data-tip={`Conversations with ${firstName(e.name)}`}>
          <span className="trunc">{title}</span>
          <Icon name="chevronDown" size={16} className="convo-pick-chev" />
        </button>
      </h1>
      <Popover anchor={anchor} open={open} onClose={() => setOpen(false)} width={320} align="start">
        <div className="menu convo-menu" role="menu" aria-label={`Conversations with ${firstName(e.name)}`}>
          {list.map((x) => {
            const word = x.status ? WORD[x.status.state] : undefined;
            return (
              <button key={x.focus} type="button" role="menuitemradio" aria-checked={x.focus === focus} className="menu-item" onClick={() => go(x.focus)}>
                <Icon name={x.icon} size={14} />
                <span className="grow trunc">{x.title}</span>
                {word ? <span className={`convo-menu-state ${x.status!.state === "needsYou" ? "status-attn" : x.status!.state === "working" ? "status-live" : ""}`}>{word}</span> : null}
              </button>
            );
          })}
          {list.length ? <div className="menu-sep" /> : null}
          <button type="button" role="menuitemradio" aria-checked={focus === null} className="menu-item" onClick={() => go(null)}>
            <Icon name="plus" size={14} /><span className="grow">New conversation</span>
          </button>
        </div>
      </Popover>
    </>
  );
}

type Entry = { label: string; icon: IconName; onSelect: () => void; hint?: string; keep?: boolean };

/**
 * Everything else you can do with a conversation, in one menu: ask where things stand, teach a new task, attach files,
 * put it away until later (or wake it), archive it with E (or reopen it), and open the employee's own page. A new
 * conversation (`rootId` null) has only what fits before its first message.
 */
export function ConversationMenu({ e, rootId, s }: { e: EmployeeView; rootId: string | null; s: ThreadState | null }) {
  const name = firstName(e.name);
  const leave = useLeave(e.id, rootId ?? "");
  const archive = useArchive(e.id, rootId ?? "");
  const anchor = useRef<HTMLButtonElement>(null);
  const list = useRef<HTMLDivElement>(null);
  const [open, setOpen] = useState(false);
  const [later, setLater] = useState(false);
  useEffect(() => {
    if (open) requestAnimationFrame(() => list.current?.querySelector<HTMLButtonElement>("button")?.focus());
  }, [open, later]);
  const close = () => { setOpen(false); setLater(false); };
  const draft = rootId ? `thread:${rootId}` : newConversationDraft(e.id);
  const busy = !!s?.runs.some(isLive);
  const snoozed = s?.status?.state === "snoozed";

  const recap = async () => {
    if (!rootId) return;
    try { await call("reply_to_run", { runId: rootId, text: WHERE_ARE_WE, archive: false, attachments: [] }); } catch (err) { toast(errorMessage(err), "err"); }
  };
  const snooze = (until: Date) => {
    if (!rootId) return;
    const go = leave();
    setMark(rootId, { snoozedUntil: until.toISOString() });
    go();
  };

  const entries: (Entry | "sep")[] = [];
  if (rootId && s?.root) entries.push({ label: "Where are we?", icon: "checklist", onSelect: () => void recap() });
  if (!busy) entries.push({ label: "Teach a new task", icon: "teach", onSelect: () => askComposer(draft, { type: "prefill", text: TEACH_PREFIX }) });
  entries.push({ label: "Attach files", icon: "paperclip", onSelect: () => askComposer(draft, { type: "attach" }) });
  if (rootId && s?.root) {
    entries.push("sep");
    if (s.archived) entries.push({ label: "Reopen", icon: "undo", onSelect: () => void reopen(rootId) });
    else {
      entries.push(snoozed
        ? { label: "Wake now", icon: "clock", onSelect: () => setMark(rootId, { snoozedUntil: undefined }) }
        : { label: "Later", icon: "clock", keep: true, onSelect: () => setLater(true) });
      // One rule: Archive shows once nothing in the thread needs you (E still files it when nothing holds it).
      if (s.ready && !s.carded && s.status?.state !== "needsYou") entries.push({ label: "Archive", icon: "archive", hint: "E", onSelect: () => void archive() });
    }
  }
  entries.push("sep", { label: `Open ${name}'s page`, icon: "org", onSelect: () => openMember(e.id, "work") });

  const presets = later ? snoozePresets(new Date()) : [];
  return (
    <>
      <button ref={anchor} type="button" className="btn btn-ghost btn-icon btn-sm" aria-label="More" data-tip="More" aria-haspopup="menu" aria-expanded={open} onClick={() => (open ? close() : setOpen(true))}>
        <Icon name="more" size={16} />
      </button>
      <Popover anchor={anchor} open={open} onClose={close} width={248}>
        <div
          ref={list} className="menu" role="menu" aria-label={later ? "Bring it back" : "Conversation"} data-esc-stop
          onKeyDown={(ev) => {
            if (ev.key !== "ArrowDown" && ev.key !== "ArrowUp") return;
            ev.preventDefault();
            const btns = [...(list.current?.querySelectorAll<HTMLButtonElement>("button:not(:disabled)") ?? [])];
            const i = btns.indexOf(document.activeElement as HTMLButtonElement);
            btns[(i + (ev.key === "ArrowDown" ? 1 : btns.length - 1)) % btns.length]?.focus();
          }}
        >
          {later ? presets.map((p) => (
            <button key={p.label} type="button" role="menuitem" className="menu-item" onClick={() => { close(); snooze(p.until); }}>
              <span className="grow">{p.label}</span>
              <span className="faint">{whenShort(p.until.toISOString()).replace(p.label === "Tomorrow" ? /^Tomorrow / : /^Today /, "")}</span>
            </button>
          )) : entries.map((it, i) => it === "sep" ? (i > 0 ? <div key={`sep-${i}`} className="menu-sep" /> : null) : (
            <button key={it.label} type="button" role="menuitem" className="menu-item" onClick={() => { if (!it.keep) close(); it.onSelect(); }}>
              <Icon name={it.icon} size={14} />
              <span className="grow">{it.label}</span>
              {it.hint === "E" ? <kbd className="kbd">E</kbd> : it.hint ? <span className="faint">{it.hint}</span> : it.keep ? <Icon name="chevronRight" size={13} className="faint" /> : null}
            </button>
          ))}
        </div>
      </Popover>
    </>
  );
}
