import { useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { call, errorMessage } from "../api/client";
import type { Card, CardReply, HireProposal, Run, Thread } from "../api/types";
import { answerCard, answering, undoAnswer, undoLatest, UNDO_MS } from "../lib/cardAnswers";
import { decisions, inboxLists, type Decision } from "../lib/decisions";
import { dayHeading, firstName, typing, whenInline } from "../lib/format";
import { refreshConversations, useOrg, useProposals, useQuery, useSkillScan } from "../lib/hooks";
import { setMark } from "../lib/marks";
import { ms, reducedMotion, useFlip } from "../lib/motion";
import { clearPending, markPending, usePending } from "../lib/pending";
import { openChat, openInbox, openRun, useRoute, type InboxView } from "../lib/router";
import { useScrollFade } from "../lib/scrollFade";
import { dismissToast, toast } from "../lib/toasts";
import { archiveAt, archivable, isLive, isOpen, threadFocus, wasArchived, type OpenCard } from "../lib/threads";
import { useShell } from "../Shell";
import { TEMPLATES } from "../components/HireSheet";
import { Icon } from "../components/Icon";
import { MenuButton, Popover, Roll, type MenuItem } from "../components/ui";
import { LaterMenu } from "../components/conversation/LaterMenu";
import { markDone, reopen, sendAndDone, SnoozeButton } from "../components/conversation/ThreadView";
import { toItem, useStatuses, type Item } from "../components/conversation/useStatuses";
import { DetailBody, InboxRow, RowSkeleton, rowView, rowWord, WORDS, type DetailContext, type ItemActions, type Row, type RowContext, type Word } from "./InboxItem";
import "./Inbox.css";

/** What the view menu calls each view. Snoozed is Later, as the button that puts things there. */
const VIEWS: { id: InboxView; label: string }[] = [
  { id: "inbox", label: "Inbox" },
  { id: "snoozed", label: "Later" },
  { id: "parked", label: "Parked" },
  { id: "archive", label: "Archive" },
];
/** One line under the head of each view but the Inbox, where its filters would be. */
const NOTES: Record<Exclude<InboxView, "inbox">, string> = {
  snoozed: "Each comes back at the time you picked.",
  parked: "Parked cards wait here, without reminders.",
  archive: "What needed nothing from you.",
};

const kbd = (k: string) => <kbd className="kbd">{k}</kbd>;

/** The keys the Inbox answers to, as ? lists them. */
const KEYS: [ReactNode, string][] = [
  [<>{kbd("J")} {kbd("K")}</>, "Next or previous item (or ↓ and ↑)"],
  [kbd("↵"), "The main action"],
  [<>{kbd("1")} to {kbd("4")}</>, "Pick that choice"],
  [kbd("L"), "Later"],
  [kbd("X"), "No, Can't, Not now or Decline"],
  [kbd("R"), "Reply in your own words"],
  [kbd("E"), "Archive, or in the Archive reopen"],
  [kbd("Z"), "Undo your last answer before it goes"],
  [kbd("Esc"), "Back to the list"],
  [kbd("?"), "Show these keys"],
];

/** One open item on the right: the current one, and while it gives way, the one leaving. */
type Layer = { id: string; phase: "none" | "fade" | "fadeout" | "out" | "in"; leaving: boolean; next: string | null };
/** How long an entry you settled stays hidden if the Inbox never hears it went; then it shows again. */
const PENDING_MAX_MS = UNDO_MS + 15_000;

/** The item last open in each view, so coming back to the Inbox opens it again. */
const lastOpen = new Map<InboxView, string>();

/** Your answer in a few words, for the toast that carries its Undo. */
function said(card: Card | undefined, reply: CardReply, who: string): string {
  const a = card?.ask;
  switch (reply.type) {
    case "choice": return `To ${who}: ${a?.type === "choices" ? a.choices[reply.index] : "your choice"}`;
    case "items": return `To ${who}: ${a?.type === "items" ? `${a.verb} ${reply.picks.filter((p) => p.tick).length}` : "your picks"}`;
    case "done": return `Done. ${who} carries on.`;
    case "cant": return `Told ${who} you can't.`;
    case "seen": return "Got it";
    case "dismiss": return "Not now";
    case "reject": return `No. ${who} drops it.`;
    case "later": return card?.timesBack ? "Parked. It won't come back by itself." : `Later. Back ${whenInline(reply.until)}.`;
    case "text": case "ask": return `To ${who}: your answer`;
  }
}

/** Puts rows that are gone (`extra`) back where they were in `before`, among those still here. */
function inPlace(before: string[], now: Row[], extra: Map<string, Row>): Row[] {
  if (!extra.size) return now;
  const out = [...now];
  for (const id of before) {
    const row = extra.get(id);
    if (!row) continue;
    const prev = before[before.indexOf(id) - 1];
    const at = prev ? out.findIndex((r) => r.id === prev) : -1;
    out.splice(at + 1, 0, row);
  }
  return out;
}

/**
 * The Inbox: what needs you, one item at a time. The list on the left holds every row, each saying in one word what it
 * needs (hire proposals first, then decisions oldest waiting first, then replies newest first); the item you are on is
 * open on the right, its evidence above and its actions in a bar at the foot, the main one on Enter. Settling an item
 * takes it out of the list at once (an answer still inside its Undo window included, with Undo in a toast), and the
 * next rises into its place. Later, Parked and the Archive are views in the list's head. Keys act on the open item;
 * an entry that leaves while the pointer is over the list keeps its place, faded, so nothing moves under it.
 */
export function InboxScreen() {
  const route = useRoute();
  const view: InboxView = route.page === "inbox" ? route.view ?? "inbox" : "inbox";
  const routeItem = route.page === "inbox" ? route.item ?? null : null;
  const { snap } = useOrg();
  const { items, ctx, error } = useStatuses();
  const allProposals = useProposals();
  const catalog = useSkillScan().catalog ?? undefined;
  const pending = usePending();
  const shell = useShell();
  const archive = useQuery(() => (view === "archive" ? call("archived") : Promise.resolve(undefined)), [view], ["run:updated"]);
  const parked = useQuery(() => call("parked_runs"), [], ["run:updated"]);
  const employees = useMemo(() => snap?.employees ?? [], [snap]);
  const people = useMemo(() => snap?.people ?? [], [snap]);
  const [filter, setFilter] = useState<Word | null>(null);
  const [keysOpen, setKeysOpen] = useState(false);
  const keysAnchor = useRef<HTMLButtonElement>(null);
  const detailRef = useRef<HTMLElement>(null);
  /** Entries this screen settled, hidden (as the badge hides them) until the Inbox no longer has them. */
  const marked = useRef(new Set<string>());
  /** Undo for answers sent together, newest last, for Z. */
  const undos = useRef<(() => void)[]>([]);
  const pressing = useRef(false);
  const lastIndex = useRef(0);
  // While the pointer is over the list, rows that leave by themselves stay where they were, faded, until it leaves.
  const over = useRef(false);
  const shownBefore = useRef<string[]>([]);
  const lastRows = useRef(new Map<string, Row>());
  const held = useRef(new Map<string, Row>());
  /** Entries you put away with Later: like your other settles, they never wait under the pointer. */
  const mine = useRef(new Set<string>());
  const [, redraw] = useState(0);

  // ---------------------------------------------------------------- the rows of each view
  const proposals = useMemo(() => (allProposals ?? []).filter((p) => p.state === "open"), [allProposals]);
  const inboxDecisions = useMemo(() => (items ? decisions(items.filter((i) => i.status.inbox)) : null), [items]);
  const snoozedDecisions = useMemo(
    () => (items ? decisions(items.filter((i) => i.status.state === "snoozed").sort((a, b) => a.status.snoozedUntil!.localeCompare(b.status.snoozedUntil!))) : null),
    [items],
  );
  const inboxRows = useMemo((): Row[] | null => {
    if (!inboxDecisions || !allProposals) return null;
    const lists = inboxLists(inboxDecisions.filter((d) => !pending.has(d.id)));
    return [
      ...proposals.filter((p) => !pending.has(p.id)).map((p): Row => ({ kind: "hire", id: p.id, p })),
      ...lists.decisions.flatMap((x): Row[] => (x.type === "group" ? x.decisions.map((d) => ({ kind: "decision", id: d.id, d, group: x })) : [{ kind: "decision", id: x.id, d: x, group: null }])),
      ...lists.replies.map((d): Row => ({ kind: "decision", id: d.id, d, group: null })),
    ];
  }, [inboxDecisions, allProposals, proposals, pending]);
  const viewRows: Row[] | null =
    view === "inbox" ? inboxRows
    : view === "snoozed" ? snoozedDecisions?.filter((d) => !pending.has(d.id)).map((d): Row => ({ kind: "decision", id: d.id, d, group: null })) ?? null
    : view === "parked" ? parked.data?.map((run): Row => ({ kind: "parked", id: run.id, run })) ?? null
    : snap && archive.data ? archive.data.flatMap((t: Thread): Row[] => {
      const item = toItem(t, snap, false, ctx);
      return item ? [{ kind: "archived", id: t.root.id, item, day: dayHeading(archiveAt(item.thread)) }] : [];
    }) : null;
  const loadError = view === "archive" ? archive.error : view === "parked" ? parked.error : error;
  const reload = view === "archive" ? archive.reload : view === "parked" ? parked.reload : refreshConversations;

  // Rows that left by themselves while the pointer was over the list keep their place; your own settles never wait.
  if (viewRows) {
    const now = new Set(viewRows.map((r) => r.id));
    if (over.current) {
      for (const id of shownBefore.current) {
        if (now.has(id) || held.current.has(id) || marked.current.has(id) || mine.current.has(id)) continue;
        const row = lastRows.current.get(id);
        if (row) held.current.set(id, row);
      }
    }
    for (const id of [...held.current.keys()]) if (now.has(id)) held.current.delete(id);
  }
  const shownRows = viewRows ? inPlace(shownBefore.current, viewRows, held.current) : null;
  useEffect(() => {
    if (!shownRows) return;
    shownBefore.current = shownRows.map((r) => r.id);
    for (const r of shownRows) lastRows.current.set(r.id, r);
  });

  // The filter by what each needs: only in the Inbox, and a filter you emptied by answering stays until you leave it.
  const counts = useMemo(() => {
    const m = new Map<Word, number>();
    for (const r of inboxRows ?? []) {
      const w = rowWord(r);
      if (w) m.set(w, (m.get(w) ?? 0) + 1);
    }
    return m;
  }, [inboxRows]);
  const activeFilter = view === "inbox" ? filter : null;
  const listed = shownRows?.filter((r) => !activeFilter || rowWord(r) === activeFilter) ?? null;
  const live = listed?.filter((r) => !held.current.has(r.id)) ?? [];
  const inboxCount = inboxRows?.length ?? 0;

  // ---------------------------------------------------------------- the open item
  const wanted = routeItem ?? lastOpen.get(view) ?? null;
  const sel = live.find((r) => r.id === wanted) ?? live[Math.min(lastIndex.current, live.length - 1)] ?? null;
  const selIndex = sel ? live.indexOf(sel) : -1;
  const selId = sel?.id ?? null;
  const allIds = useMemo(() => new Set((viewRows ?? []).map((r) => r.id)), [viewRows]);

  useEffect(() => {
    if (selIndex >= 0) lastIndex.current = selIndex;
    if (!selId) return;
    lastOpen.set(view, selId);
    if (route.page === "inbox" && selId !== routeItem) openInbox(view, selId, true);
  }, [selId, selIndex, view, routeItem, route.page]);

  // The open item gives way to the next, the two overlapping so no frame is empty: when it left the list (settled) it
  // rises out as the next rises in from just below; when you moved, a quick crossfade; the first one simply shows. The
  // leaving one keeps what it showed, its own Next included.
  const [layers, setLayers] = useState<Layer[]>([]);
  const ghosts = useRef(new Map<string, Row>());
  const settledByKeys = useRef(false);
  const lastNext = useRef<string | null>(null);
  const current = layers.find((l) => !l.leaving) ?? null;
  useLayoutEffect(() => {
    const next = sel?.id ?? null;
    const cur = current?.id ?? null;
    if (next === cur) return;
    if (!cur || !next || !viewRows) { setLayers(next ? [{ id: next, phase: "none", leaving: false, next: null }] : []); return; }
    const settled = !allIds.has(cur) && !held.current.has(cur);
    const leaving: Layer = { id: cur, phase: settled ? "out" : "fadeout", leaving: true, next: lastNext.current };
    setLayers([leaving, { id: next, phase: settled ? "in" : "fade", leaving: false, next: null }]);
    if (settled && settledByKeys.current) focusRow(next);
    const t = window.setTimeout(() => setLayers((ls) => ls.filter((l) => l !== leaving)), settled ? 220 : 160);
    return () => window.clearTimeout(t);
  }, [sel?.id]); // eslint-disable-line react-hooks/exhaustive-deps
  const rowOf = (id: string) => shownRows?.find((r) => r.id === id) ?? ghosts.current.get(id) ?? null;
  for (const l of layers) {
    const r = shownRows?.find((x) => x.id === l.id);
    if (r) ghosts.current.set(l.id, r);
  }
  for (const id of [...ghosts.current.keys()]) if (!layers.some((l) => l.id === id)) ghosts.current.delete(id);
  const shownRow = current ? rowOf(current.id) : null;
  const shownIndex = shownRow ? live.findIndex((r) => r.id === shownRow.id) : -1;
  const nextRow = shownIndex >= 0 ? live[shownIndex + 1] ?? null : null;
  const nextTitle = nextRow ? rowView(nextRow, { employees, people }).title : null;
  lastNext.current = nextTitle;

  // Your settles leave the list at once; they stay out until the Inbox no longer has them, or come back.
  const liveIds = useMemo(() => {
    if (!inboxDecisions || !allProposals) return null;
    return new Set([...proposals.map((p) => p.id), ...inboxDecisions.map((d) => d.id), ...(snoozedDecisions ?? []).map((d) => d.id)]);
  }, [inboxDecisions, snoozedDecisions, proposals, allProposals]);
  useEffect(() => {
    if (!liveIds) return;
    for (const id of [...marked.current]) {
      if (liveIds.has(id)) continue;
      marked.current.delete(id);
      clearPending(id);
    }
  }, [liveIds]);

  const select = (id: string, byKeys = false) => {
    if (!live.some((r) => r.id === id) && shownRows?.some((r) => r.id === id)) setFilter(null);
    openInbox(view, id, true);
    if (byKeys) focusRow(id);
  };
  const move = (by: number) => {
    const at = sel ? live.indexOf(sel) : -1;
    const to = live[Math.max(0, Math.min(live.length - 1, at + by))];
    if (to && to.id !== sel?.id) select(to.id, true);
  };
  const pickView = (v: InboxView) => {
    over.current = false;
    held.current.clear();
    setFilter(null);
    openInbox(v, lastOpen.get(v) ?? null);
  };

  /** Settles entries `ids`: they leave the list at once and the next rises into place, while `go` runs. `text` and
   *  `undo` make a toast with Undo that lasts the Undo window. If `go` resolves false (undone, or it failed) they
   *  come back, and the first is opened again. */
  const settle = (ids: string[], go: () => Promise<boolean>, opts: { text?: string; undo?: () => void } = {}) => {
    for (const id of ids) {
      marked.current.add(id);
      markPending(id);
    }
    window.setTimeout(() => {
      for (const id of ids) if (marked.current.delete(id)) clearPending(id);
    }, PENDING_MAX_MS);
    const t = opts.text ? toast(opts.text, "ok", opts.undo ? { label: "Undo", run: opts.undo } : undefined, { ms: UNDO_MS }) : null;
    void go().then((ok) => {
      if (ok) return;
      for (const id of ids) {
        marked.current.delete(id);
        clearPending(id);
      }
      if (t !== null) dismissToast(t);
      openInbox(view, ids[0], true);
    });
  };

  const actionsFor = (d: Decision<Item>): ItemActions => {
    const each = d.type === "failures" ? d.failures : [d];
    const name = (run: Run) => firstName(employees.find((x) => x.id === run.employeeId)?.name ?? d.item.employee.name);
    const who = name(d.run);
    return {
      done: () => settle([d.id], () => markDone(...each.filter((x) => archivable(x.thread, x.item.thread)).map((x) => x.thread.root.id))),
      sendDone: async (text, attachments) => {
        settle([d.id], async () => {
          try {
            await sendAndDone(d.thread.root.id, who, text, attachments);
            return true;
          } catch (err) {
            toast(errorMessage(err), "err");
            return false;
          }
        });
      },
      // A run asking several things stays until its last card is answered; an answer still on its way counts, and
      // news asks nothing. Another's card under the same question answers that one only, so the entry stays.
      answer: async (run, index, reply) => {
        const last = (run === d.run || (d.type === "card" && d.cards.length === 1))
          && !run.cards.some((c, i) => i !== index && isOpen(c) && !answering(run.id, i));
        if (!last) {
          await answerCard(run, index, reply, name(run));
          return;
        }
        // Quiet: the settle's own toast, with its Undo, is the only one.
        settle([d.id], () => answerCard(run, index, reply, name(run), { quiet: true }), { text: said(run.cards[index], reply, name(run)), undo: () => undoAnswer(run.id, index) });
      },
      answerMany: (picks, ids) => {
        let cancel = () => {};
        const go = () => new Promise<boolean>((done) => {
          const timer = window.setTimeout(() => done(true), UNDO_MS);
          cancel = () => { window.clearTimeout(timer); done(false); };
          undos.current.push(cancel);
        }).then(async (ok) => {
          undos.current = undos.current.filter((u) => u !== cancel);
          if (!ok) return false;
          let sent = 0;
          for (const p of picks) {
            try {
              await call("answer_card", { runId: p.card.run.id, card: p.card.index, answer: p.reply });
              sent++;
            } catch (err) {
              toast(errorMessage(err), "err");
            }
          }
          if (sent) toast(`Sent ${who} ${sent} ${sent === 1 ? "answer" : "answers"}.`, "ok");
          return sent > 0;
        });
        settle(ids, go, { text: `${picks.length === 2 ? "Both answers" : `${picks.length} answers`} to ${who}`, undo: () => cancel() });
      },
    };
  };

  /** Later on an item that is not one card: the whole row goes away until `until`, with Undo. */
  const snooze = (d: Decision<Item>, until: Date) => {
    const rows = [...new Set((d.type === "failures" ? d.failures : [d]).map((x) => x.item.thread.root.id))];
    mine.current.add(d.id);
    for (const root of rows) setMark(root, { snoozedUntil: until.toISOString() });
    toast(`Later. Back ${whenInline(until.toISOString())}.`, "ok", { label: "Undo", run: () => { for (const root of rows) setMark(root, { snoozedUntil: undefined }); } });
  };
  const wake = (d: Decision<Item>) => {
    for (const root of new Set((d.type === "failures" ? d.failures : [d]).map((x) => x.item.thread.root.id))) setMark(root, { snoozedUntil: undefined });
  };

  const detailCtx: DetailContext = {
    employees, people, snap, catalog, actionsFor,
    answerParked: async (run, index, reply) => { await answerCard(run, index, reply, firstName(employees.find((x) => x.id === run.employeeId)?.name ?? "them")); },
    settle: (ids, go) => settle(ids, go),
    alsoFrom: (employeeId, except) => (inboxRows ?? []).filter((r) => r.id !== except && r.kind === "decision" && r.d.run.employeeId === employeeId).map((r) => {
      const v = rowView(r, { employees, people });
      return { id: r.id, title: v.title, word: v.word, age: v.side };
    }),
    select: (id) => select(id),
    reviewProposal: (p: HireProposal, from) => shell.reviewProposal(p, from),
    reopen: (id) => void reopen(id),
  };

  // ---------------------------------------------------------------- keys
  // J and K (or the arrows) move; Enter presses the main action (A too); 1 to 4 pick a choice; L is Later;
  // X is No, Can't, Not now or Decline; R replies; E archives, or in the Archive reopens; Z takes back your newest
  // answer still waiting to go; Escape goes back to the list; ? lists the keys. Nothing acts while you type or while
  // a sheet is open.
  const latest = useRef({ move, sel, shownRow, detailCtx });
  latest.current = { move, sel, shownRow, detailCtx };
  useEffect(() => {
    const press = (el: HTMLElement | null | undefined) => {
      if (!el || pressing.current) return !!el;
      if (reducedMotion()) {
        settledByKeys.current = true;
        el.click();
        window.setTimeout(() => { settledByKeys.current = false; }, 600);
        return true;
      }
      pressing.current = true;
      el.animate([{ transform: "scale(1)" }, { transform: "scale(0.97)", offset: 0.4 }, { transform: "scale(1)" }], { duration: 200, easing: "cubic-bezier(0.3, 0, 0, 1)" });
      window.setTimeout(() => {
        pressing.current = false;
        settledByKeys.current = true;
        el.click();
        // Only a settle that follows at once moves focus on to the next row.
        window.setTimeout(() => { settledByKeys.current = false; }, 600);
      }, 120);
      return true;
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented || e.ctrlKey || e.metaKey || e.altKey || typing(e.target) || document.querySelector("[role=dialog]")) return;
      const L = latest.current;
      const detail = detailRef.current;
      const active = document.activeElement instanceof HTMLElement ? document.activeElement : null;
      const inDetail = !!detail && !!active && detail.contains(active);
      const q = (s: string) => detail?.querySelector<HTMLElement>(s) ?? null;
      // The main action: the bar's own, else the card's in the bar (its buttons sit in .d-slot: its black button, or for a
      // choice the employee's pick), else in the card.
      const main = () => q("[data-main]:not(:disabled)") ?? q(".d-slot .btn-primary:not(:disabled)") ?? q(".d-slot .card-pick:not(:disabled)")
        ?? q(":is(.d-card-main, .d-thread .tp) .btn-primary:not(:disabled)");
      const key = e.key.length === 1 ? e.key.toLowerCase() : e.key;
      switch (key) {
        case "j": case "k": case "ArrowDown": case "ArrowUp":
          // In the open item the arrows scroll it, as usual.
          if (key.startsWith("Arrow") && inDetail) return;
          L.move(key === "j" || key === "ArrowDown" ? 1 : -1);
          break;
        case "Enter":
          // A focused button in the open item answers Enter itself.
          if (inDetail && active?.closest("button, a, [role=menuitem]")) return;
          if (!press(main())) return;
          break;
        case "a":
          if (!press(main())) return;
          break;
        case "1": case "2": case "3": case "4":
          if (!press(q(`:is(.d-slot, .d-card-main) [data-card-choice="${Number(key) - 1}"]:not(:disabled)`))) return;
          break;
        case "l": {
          const later = q(".d-bar [data-later], .d-bar [data-card-later]");
          if (!later || (later as HTMLButtonElement).disabled) return;
          later.click();
          break;
        }
        case "x":
          if (!press(q(":is(.d-slot, .d-card-main) [data-card-no]:not(:disabled)") ?? q("[data-no]:not(:disabled)"))) return;
          break;
        case "z": {
          const u = undos.current.pop();
          if (u) u();
          else if (!undoLatest()) return;
          break;
        }
        case "r": {
          const reply = q("[data-card-reply]") ?? q(".d-thread textarea");
          if (!reply) return;
          reply.focus();
          break;
        }
        case "e": {
          const row = L.shownRow;
          if (row?.kind === "archived") {
            if (!wasArchived(row.item.thread)) return;
            L.detailCtx.reopen(row.item.thread.root.id);
            break;
          }
          if (row?.kind !== "decision") return;
          const d = row.d;
          const ok = d.type === "failures" ? d.failures.some((f) => archivable(f.thread, f.item.thread))
            : d.type !== "card" && d.type !== "blocked" && d.type !== "proposal" && archivable(d.thread, d.item.thread);
          if (!ok) return;
          L.detailCtx.actionsFor(d).done();
          break;
        }
        case "Escape":
          if (!inDetail || !L.sel) return;
          focusRow(L.sel.id);
          break;
        case "?":
          setKeysOpen(true);
          break;
        default:
          return;
      }
      e.preventDefault();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  // ---------------------------------------------------------------- the page
  const loading = !viewRows && !loadError;
  const emptied = !!activeFilter && listed?.length === 0 && inboxCount > 0;
  const snoozedNext = snoozedDecisions?.find((d) => d.item.status.snoozedUntil) ?? null;
  const viewCount = viewRows?.length ?? 0;
  const viewTitle = VIEWS.find((v) => v.id === view)!.label;
  // Each view with how many it holds; the Archive's once it has been read.
  const counts_: Record<InboxView, number | null> = {
    inbox: inboxRows ? inboxCount : null,
    snoozed: snoozedDecisions?.length ?? null,
    parked: parked.data?.length ?? null,
    archive: archive.data?.length ?? null,
  };
  const viewItems: ViewItem[] = VIEWS.map((v) => ({ label: v.label, count: counts_[v.id], current: v.id === view, onSelect: () => pickView(v.id) }));

  // Nothing to list: one pane, its title, and a card saying what this is for and one way on. A filter you emptied
  // keeps its chips.
  if (!loading && (loadError || (listed && listed.length === 0))) {
    let card: ReactNode;
    if (loadError) {
      card = (
        <EmptyCard title={`Couldn't load ${view === "archive" ? "the Archive" : view === "parked" ? "Parked" : view === "snoozed" ? "Later" : "the Inbox"}`} alert>
          <p>{loadError}</p>
          <div className="ibx-empty-acts"><button type="button" className="btn btn-primary" onClick={reload}><Icon name="refresh" size={14} /> Try again</button></div>
        </EmptyCard>
      );
    } else if (emptied && activeFilter) {
      card = (
        <EmptyCard title={`That was the last one to ${activeFilter}`}>
          <p>{inboxCount} more {inboxCount === 1 ? "waits" : "wait"} in All.</p>
          <div className="ibx-empty-acts"><button type="button" className="btn btn-primary" onClick={() => setFilter(null)}>Show all {inboxCount}</button></div>
        </EmptyCard>
      );
    } else {
      card = <EmptyView view={view} hired={employees.length > 0} snoozedNext={snoozedNext} laterCount={snoozedDecisions?.length ?? 0} onView={pickView} onHire={(template, from) => shell.hire(template, from)} />;
    }
    return (
      <div className="ibx-one">
        <header className="ilist-head ibx-one-head">
          <TitleMenu title={viewTitle} count={emptied ? inboxCount : loadError ? null : 0} items={viewItems} />
        </header>
        {emptied ? <Chips counts={counts} total={inboxCount} active={activeFilter} onPick={setFilter} loading={false} /> : null}
        <div className="ibx-one-body">{card}</div>
      </div>
    );
  }

  return (
    <div className="ibx">
      <section className="ilist" aria-label={viewTitle}>
        <header className="ilist-head">
          <TitleMenu title={viewTitle} count={viewRows ? viewCount : null} items={viewItems} />
        </header>
        {view === "inbox" ? (
          <Chips counts={counts} total={inboxCount} active={activeFilter} onPick={setFilter} loading={!inboxRows} />
        ) : (
          <p className="ilist-note">{NOTES[view]}</p>
        )}
        {loading ? (
          <div className="irows" aria-hidden>{[64, 78, 56, 70, 60, 74, 52, 66, 72].map((w, i) => <RowSkeleton key={i} w={w} />)}</div>
        ) : listed ? (
          <RowList
            key={`${view}:${activeFilter ?? "all"}`} rows={listed} selId={sel?.id ?? null} held={held.current}
            ctx={{ employees, people }} onSelect={(id) => select(id)}
            onEnter={() => { over.current = true; }}
            onLeave={() => {
              over.current = false;
              if (!held.current.size) return;
              held.current.clear();
              redraw((n) => n + 1);
            }}
          />
        ) : <div className="irows" />}
      </section>

      <section ref={detailRef} className="ibx-detail" aria-label="Open item">
        {shownRow ? (
          <>
            <div className="d-bar">
              <span className="d-num num">
                <Roll value={(shownIndex >= 0 ? shownIndex : Math.max(selIndex, 0)) + 1} /> of <Roll value={live.length} />
              </span>
              <span className="d-nav">
                <button type="button" className="btn btn-ghost btn-icon btn-sm" data-tip="Previous" data-tip-key="K" aria-label="Previous (K)" disabled={shownIndex <= 0} onClick={() => move(-1)}>
                  <Icon name="chevronUp" size={16} />
                </button>
                <button type="button" className="btn btn-ghost btn-icon btn-sm" data-tip="Next" data-tip-key="J" aria-label="Next (J)" disabled={shownIndex < 0 || shownIndex >= live.length - 1} onClick={() => move(1)}>
                  <Icon name="chevronDown" size={16} />
                </button>
              </span>
              <span className="grow" />
              <LaterControl row={shownRow} ctx={detailCtx} onSnooze={snooze} onWake={wake} />
              <button
                ref={keysAnchor} type="button" className="btn btn-ghost btn-icon btn-sm" aria-label="Keyboard shortcuts"
                data-tip={keysOpen ? undefined : "Keyboard shortcuts"} data-tip-key={keysOpen ? undefined : "?"}
                aria-haspopup="dialog" aria-expanded={keysOpen} onClick={() => setKeysOpen((k) => !k)}
              >
                <Icon name="help" size={16} />
              </button>
              <Popover anchor={keysAnchor} open={keysOpen} onClose={() => setKeysOpen(false)} width={320}>
                <dl className="ibx-keys" aria-label="Keyboard shortcuts">
                  {KEYS.map(([k, does], i) => <div key={i}><dt>{k}</dt><dd>{does}</dd></div>)}
                </dl>
              </Popover>
              <MenuButton items={moreFor(shownRow, detailCtx)} label="More for this item" />
            </div>
            <div className="d-stage">
              {layers.map((l) => {
                const row = rowOf(l.id);
                return row ? (
                  <Swap key={l.id} row={row} phase={l.phase} leaving={l.leaving} ctx={detailCtx} next={l.leaving ? l.next : nextTitle} onNext={() => move(1)} />
                ) : null;
              })}
            </div>
          </>
        ) : <DetailSkeleton />}
        <p className="ibx-sr" aria-live="polite">{current?.phase === "in" && shownRow ? `Next: ${rowView(shownRow, { employees, people }).title}` : ""}</p>
      </section>
    </div>
  );
}

/** Focuses a row of the list. The list brings the open row into view itself, clear of its edges. */
function focusRow(id: string) {
  document.querySelector<HTMLElement>(`[data-row="${CSS.escape(id)}"]`)?.focus({ preventScroll: true });
}

/** The ⋯ menu's actions for the open item: the conversation it came from. */
function moreFor(row: Row, ctx: DetailContext): MenuItem[] {
  const talk = (employeeId: string, focus: string): MenuItem[] =>
    ctx.employees.some((x) => x.id === employeeId) ? [{ label: "Open the conversation", icon: "chat", onSelect: () => openChat(employeeId, focus) }] : [];
  if (row.kind === "hire") {
    const runId = row.p.runId;
    return runId ? [{ label: "Open the conversation that asked", icon: "chat", onSelect: () => openRun(runId) }] : [];
  }
  if (row.kind === "decision") {
    const d = row.d;
    return d.type === "failures" ? [{ label: "Open the conversation", icon: "chat", onSelect: () => openRun(d.run.id) }] : talk(d.run.employeeId, threadFocus(d.thread.root.id));
  }
  if (row.kind === "parked") return talk(row.run.employeeId, threadFocus(row.run.id));
  return talk(row.item.employee.id, threadFocus(row.item.thread.root.id));
}

/** Later in the bar: a card's own Later (it comes back, and once it has, Later parks it), or Later for the whole row
 *  (Wake now while it is away). None for a hire proposal, a parked card or the Archive. */
function LaterControl({ row, ctx, onSnooze, onWake }: {
  row: Row; ctx: DetailContext; onSnooze: (d: Decision<Item>, until: Date) => void; onWake: (d: Decision<Item>) => void;
}) {
  if (row.kind !== "decision") return null;
  const d = row.d;
  // While it is away (in Later), Wake now brings it back.
  if (d.type === "card" && d.cards.length === 1 && !d.item.status.snoozedUntil) {
    const c: OpenCard = d.cards[0];
    const t = c.card.ask.type;
    if (!c.card.parked && (t === "choices" || t === "items" || t === "task" || t === "question")) {
      return <LaterMenu timesBack={c.card.timesBack} disabled={isLive(c.run)} onLater={(until) => void ctx.actionsFor(d).answer(c.run, c.index, { type: "later", until: until.toISOString() })} />;
    }
  }
  return <SnoozeButton snoozedUntil={d.item.status.snoozedUntil} onSnooze={(until) => onSnooze(d, until)} onWake={() => onWake(d)} />;
}

/** The open item's body and its action bar, giving way to the next as one: out with a short rise when it was settled
 *  as the next rises in from just below, or a quick crossfade when you moved. A leaving one lies under the next, inert. */
function Swap({ row, phase, leaving, ctx, next, onNext }: { row: Row; phase: Layer["phase"]; leaving: boolean; ctx: DetailContext; next: string | null; onNext: () => void }) {
  const [foot, setFoot] = useState<HTMLDivElement | null>(null);
  const [slot, setSlot] = useState<HTMLDivElement | null>(null);
  const fade = useScrollFade<HTMLDivElement>();
  const cls = { none: "", fade: "d-fade-in", fadeout: "d-fade-out", out: "leave-rise", in: "enter-rise" }[phase];
  return (
    <div className={`d-swap ${cls} ${leaving ? "d-swap-leaving" : ""}`} inert={leaving || undefined} aria-hidden={leaving || undefined}>
      <div ref={fade} className="d-body scroll-fade">
        <DetailBody row={row} ctx={ctx} foot={foot} slot={slot} />
      </div>
      <div className="d-foot">
        {/* The open card's own buttons (CardView's actionsSlot), then the item's actions. */}
        <div className="d-slot cv-slot" ref={setSlot} />
        <div className="d-acts" ref={setFoot} />
        {next ? (
          <button type="button" className="d-next" onClick={onNext}>
            <span className="d-next-t">Next: {next}</span>{kbd("J")}
          </button>
        ) : null}
      </div>
    </div>
  );
}

/** The list: rows that move rather than swap. The open row's grey is one highlight under the rows, in the open row's
 *  slot: when you settle a row it stays where it is (only its height changes) while the row closes and the next row
 *  rises into it; when you move, it slides. A new row drops in as its room opens. The open row is kept in view, clear
 *  of the edges' fades. Day headings in the Archive. */
function RowList({ rows, selId, held, ctx, onSelect, onEnter, onLeave }: {
  rows: Row[]; selId: string | null; held: Map<string, Row>; ctx: RowContext;
  onSelect: (id: string) => void; onEnter: () => void; onLeave: () => void;
}) {
  const shown = useLeaving(rows);
  // Only rows that stay count as a change of order: a leaving row closes its own height, and taking it out afterwards
  // moves nothing, so it must not replay the move.
  const liveKey = shown.filter((x) => !x.leaving).map((x) => x.row.id).join(",");
  const flip = useFlip<HTMLDivElement>(liveKey);
  const fade = useScrollFade<HTMLDivElement>();
  const hl = useRef<HTMLDivElement>(null);
  const placed = useRef<string | null>(null);
  const leavingN = shown.filter((x) => x.leaving).length;

  useLayoutEffect(() => {
    const host = flip.current;
    const box = hl.current;
    if (!host || !box) return;
    const moved = placed.current !== selId;
    const place = (how: "move" | "settle" | "still"): { top: number; height: number } | null => {
      const el = selId ? host.querySelector<HTMLElement>(`[data-row="${CSS.escape(selId)}"]`) : null;
      if (!el || el.classList.contains("irow-held")) { box.style.opacity = "0"; return null; }
      const h = host.getBoundingClientRect();
      const r = el.getBoundingClientRect();
      const moveInFlight = new DOMMatrixReadOnly(getComputedStyle(el.closest(".irow-w") ?? el).transform);
      const landsAt = r.top - moveInFlight.m42;
      // Where the row will be once the rows closing above it have closed.
      let top = landsAt - h.top + host.scrollTop;
      for (const w of host.querySelectorAll<HTMLElement>('.irow-w[data-open="false"]')) {
        const wr = w.getBoundingClientRect();
        if (wr.bottom <= landsAt + 1) top -= wr.height;
      }
      const ridesTheRowsMove = !moved && (moveInFlight.m41 !== 0 || moveInFlight.m42 !== 0);
      // Under reduced motion the rows fade, then step at once; the highlight takes its new height on that step.
      box.style.transition = how === "still" ? "none"
        : how === "settle" ? (reducedMotion() ? "transform 1ms linear, height 1ms linear var(--d-fade)" : "transform var(--d-ui) var(--ease), height var(--d-settle) var(--ease-move)")
        : ridesTheRowsMove ? "transform var(--d-arrive) var(--ease-move), height var(--d-ui) var(--ease)"
        : "transform var(--d-ui) var(--ease), height var(--d-ui) var(--ease)";
      box.style.transform = `translate(${r.left - moveInFlight.m41 - h.left}px, ${top}px)`;
      box.style.width = `${r.width}px`;
      box.style.height = `${r.height}px`;
      box.style.opacity = "1";
      return { top, height: r.height };
    };
    const at = place(placed.current === null ? "still" : leavingN ? "settle" : "move");
    placed.current = selId;
    // Keep the open row in view, a row's room clear of the fades at either edge.
    if (moved && at) {
      const room = 56;
      if (at.top < host.scrollTop + room) host.scrollTop = Math.max(0, at.top - room);
      else if (at.top + at.height > host.scrollTop + host.clientHeight - room) host.scrollTop = at.top + at.height - host.clientHeight + room;
    }
    // A ResizeObserver reports once as it starts watching; only a real resize re-places the highlight at once.
    let seen = false;
    const ro = new ResizeObserver(() => { if (seen) place("still"); seen = true; });
    ro.observe(host);
    return () => ro.disconnect();
  }, [selId, liveKey, leavingN, flip]);

  // The open row's grey already says which row J and K are on; the focus ring shows only when Tab brought you here.
  useEffect(() => {
    const host = flip.current;
    if (!host) return;
    const key = (e: KeyboardEvent) => { if (e.key === "Tab") host.dataset.tab = ""; else delete host.dataset.tab; };
    const pointer = () => { delete host.dataset.tab; };
    window.addEventListener("keydown", key, true);
    window.addEventListener("pointerdown", pointer, true);
    return () => { window.removeEventListener("keydown", key, true); window.removeEventListener("pointerdown", pointer, true); };
  }, [flip]);

  let day = "";
  return (
    <div
      ref={(el) => { flip.current = el; fade.current = el; }}
      className="irows inbox-list scroll-fade" role="listbox" aria-label="Items" onMouseEnter={onEnter} onMouseLeave={onLeave}
    >
      <div ref={hl} className="irow-hl" aria-hidden />
      {shown.map(({ row, leaving }) => {
        const heading = row.kind === "archived" && row.day !== day ? row.day : null;
        if (row.kind === "archived") day = row.day;
        const on = !leaving && row.id === selId;
        return (
          <div key={row.id} data-key={row.id} className={`irow-w collapse ${on ? "irow-w-on" : ""}`} data-open={!leaving}>
            <div>
              {heading ? <div className="irow-day">{heading}</div> : null}
              <InboxRow id={row.id} view={rowView(row, ctx)} selected={on} held={held.has(row.id)} onSelect={() => onSelect(row.id)} />
            </div>
          </div>
        );
      })}
    </div>
  );
}

/** Keeps rows that left mounted, where they were, while they close (`--d-settle`, or a fade then a snap under reduced
 *  motion), then lets them go. */
function useLeaving(rows: Row[]): { row: Row; leaving: boolean }[] {
  const prev = useRef<Row[]>([]);
  const gone = useRef(new Map<string, Row>());
  const timers = useRef(new Map<string, number>());
  const [, tick] = useState(0);
  const now = new Set(rows.map((r) => r.id));
  for (const r of prev.current) if (!now.has(r.id) && !gone.current.has(r.id)) gone.current.set(r.id, r);
  for (const id of [...gone.current.keys()]) if (now.has(id)) gone.current.delete(id);
  const merged = inPlace(prev.current.map((r) => r.id), rows, gone.current);
  useEffect(() => {
    prev.current = merged;
    // Each leaving row has one timer from when it started to leave; later renders never restart it.
    for (const [id, t] of timers.current) {
      if (gone.current.has(id)) continue;
      window.clearTimeout(t);
      timers.current.delete(id);
    }
    const wait = (reducedMotion() ? ms("--d-fade") : ms("--d-settle")) + 40;
    for (const id of gone.current.keys()) {
      if (timers.current.has(id)) continue;
      timers.current.set(id, window.setTimeout(() => {
        timers.current.delete(id);
        gone.current.delete(id);
        prev.current = prev.current.filter((r) => r.id !== id);
        tick((n) => n + 1);
      }, wait || 420));
    }
  });
  useEffect(() => () => { for (const t of timers.current.values()) window.clearTimeout(t); }, []);
  return merged.map((row) => ({ row, leaving: gone.current.has(row.id) }));
}

/** The filters by what each item needs: All, then each word present, with its count. Those that don't fit go under
 *  More. A filter you emptied keeps its chip until you leave it. */
function Chips({ counts, total, active, onPick, loading }: { counts: Map<Word, number>; total: number; active: Word | null; onPick: (w: Word | null) => void; loading: boolean }) {
  const words = WORDS.filter((w) => (counts.get(w) ?? 0) > 0 || w === active);
  const box = useRef<HTMLDivElement>(null);
  const ruler = useRef<HTMLDivElement>(null);
  const [fit, setFit] = useState(words.length);
  const moreAnchor = useRef<HTMLButtonElement>(null);
  const [moreOpen, setMoreOpen] = useState(false);
  const key = words.map((w) => `${w}${counts.get(w) ?? 0}`).join(",");
  // A hidden copy of every chip is measured; as many as fit show, with room kept for More when some don't.
  useLayoutEffect(() => {
    const el = box.current;
    const r = ruler.current;
    if (!el || !r) return;
    const measure = () => {
      const widths = [...r.children].map((c) => (c as HTMLElement).offsetWidth);
      const gap = 6;
      const room = el.clientWidth;
      const all = widths.reduce((s, w) => s + w + gap, -gap);
      if (all <= room) { setFit(words.length); return; }
      let used = widths[0] + gap + 76;
      let n = 0;
      for (const w of widths.slice(1)) {
        if (used + w + gap > room) break;
        used += w + gap;
        n++;
      }
      setFit(n);
    };
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, [key]); // eslint-disable-line react-hooks/exhaustive-deps
  if (loading) return <div className="ilist-chips" />;
  const hidden = words.slice(fit);
  const hiddenActive = active && hidden.includes(active) ? active : null;
  const chip = (w: Word | null, n: number) => (
    <button key={w ?? "all"} type="button" className="chip" aria-pressed={active === w} onClick={() => onPick(w)}>
      {w ?? "All"} <span className="n num"><Roll value={n} /></span>
    </button>
  );
  return (
    <div className="ilist-chips" ref={box} role="toolbar" aria-label="Show only">
      <div className="ilist-ruler" ref={ruler} aria-hidden>
        {[null, ...words].map((w) => <span key={w ?? "all"} className="chip">{w ?? "All"} <span className="n">{w ? counts.get(w) ?? 0 : total}</span></span>)}
      </div>
      {chip(null, total)}
      {words.slice(0, fit).map((w) => chip(w, counts.get(w) ?? 0))}
      {hidden.length ? (
        <>
          <button ref={moreAnchor} type="button" className="chip" aria-pressed={!!hiddenActive} aria-haspopup="menu" aria-expanded={moreOpen} onClick={() => setMoreOpen((o) => !o)}>
            {hiddenActive ?? "More"} <Icon name="chevronDown" size={12} />
          </button>
          <Popover anchor={moreAnchor} open={moreOpen} onClose={() => setMoreOpen(false)} width={180} align="end">
            <div className="menu" role="menu" aria-label="More filters">
              {hidden.map((w) => (
                <button key={w} type="button" role="menuitem" className="menu-item" onClick={() => { setMoreOpen(false); onPick(w); }}>
                  <span className="grow">{w}</span><span className="faint num">{counts.get(w) ?? 0}</span>
                </button>
              ))}
            </div>
          </Popover>
        </>
      ) : null}
    </div>
  );
}

type ViewItem = { label: string; count: number | null; current: boolean; onSelect: () => void };

/** The list's title, which is also the switch between views: the view's name and how many it holds. */
function TitleMenu({ title, count, items }: { title: string; count: number | null; items: ViewItem[] }) {
  const anchor = useRef<HTMLButtonElement>(null);
  const [open, setOpen] = useState(false);
  return (
    <>
      <h1 className="ilist-h1">
        <button ref={anchor} type="button" className="ilist-title ilist-switch" aria-haspopup="menu" aria-expanded={open} onClick={() => setOpen((o) => !o)}>
          {title}{count ? <span className="ilist-n num"><Roll value={count} /></span> : null}
          <Icon name="chevronDown" size={16} className="ilist-chev" />
        </button>
      </h1>
      <Popover anchor={anchor} open={open} onClose={() => setOpen(false)} width={200} align="start">
        <div className="menu" role="menu" aria-label="Show">
          {items.map((it) => (
            <button key={it.label} type="button" role="menuitemradio" aria-checked={it.current} className="menu-item ilist-menu-item" onClick={() => { setOpen(false); it.onSelect(); }}>
              <span className="ilist-menu-check">{it.current ? <Icon name="check" size={13} /> : null}</span>
              <span className="grow">{it.label}</span>
              {it.count !== null ? <span className="faint num">{it.count}</span> : null}
            </button>
          ))}
        </div>
      </Popover>
    </>
  );
}

/** A view with nothing in it, as page type in the upper third (the parts sheet's empty pattern, without a box): what
 *  this is for, then one way on. */
function EmptyCard({ title, alert = false, children }: { title: string; alert?: boolean; children: ReactNode }) {
  return (
    <div className={`empty ibx-empty ${alert ? "ibx-empty-alert" : ""}`} role={alert ? "alert" : undefined}>
      <h2 className="empty-title">{alert ? <Icon name="alert" size={20} /> : null}{title}</h2>
      {children}
    </div>
  );
}

/** What an empty view says: what it is for, and one way on. */
function EmptyView({ view, hired, snoozedNext, laterCount, onView, onHire }: {
  view: InboxView; hired: boolean; snoozedNext: Decision<Item> | null; laterCount: number; onView: (v: InboxView) => void; onHire: (template: string | null, from: HTMLElement | null) => void;
}) {
  const hireBtn = useRef<HTMLButtonElement>(null);
  if (view === "inbox" && !hired) {
    return (
      <EmptyCard title="When your team needs you, it lands here">
        <p>Decisions, questions, hires and anything blocked, one at a time, with the main answer on Enter. Hire someone to start.</p>
        <div className="ibx-empty-roles" role="group" aria-label="Start from a role">
          {TEMPLATES.filter((t) => t.id !== "blank").map((t) => (
            <button key={t.id} type="button" className="chip" onClick={(ev) => onHire(t.id, ev.currentTarget)}>{t.title}</button>
          ))}
        </div>
        <div className="ibx-empty-acts"><button ref={hireBtn} type="button" className="btn btn-primary" onClick={() => onHire(null, hireBtn.current)}><Icon name="userPlus" size={14} /> Hire an employee</button></div>
      </EmptyCard>
    );
  }
  if (view === "inbox") {
    const back = snoozedNext?.item.status.snoozedUntil;
    return (
      <EmptyCard title="Nothing needs you">
        <p>{back ? `${laterCount === 1 ? "One item comes" : `${laterCount} wait in Later; the first comes`} back ${whenInline(back)}.` : "New decisions land here as your team works."}</p>
        <div className="ibx-empty-acts">
          {back ? <button type="button" className="btn" onClick={() => onView("snoozed")}><Icon name="clock" size={14} /> Open Later</button> : null}
          <button type="button" className="btn" onClick={() => onView("archive")}><Icon name="archive" size={14} /> Open the Archive</button>
        </div>
      </EmptyCard>
    );
  }
  const [title, text] =
    view === "snoozed" ? ["Nothing for later", "Later puts an item away until a time you pick, and it comes back by itself."] as const
    : view === "parked" ? ["Nothing parked", "A card put off twice waits here until you answer it."] as const
    : ["Nothing in the Archive", "What needs nothing from you lands here, with what you archive."] as const;
  return (
    <EmptyCard title={title}>
      <p>{text}</p>
      <div className="ibx-empty-acts"><button type="button" className="btn" onClick={() => onView("inbox")}>Back to the Inbox</button></div>
    </EmptyCard>
  );
}

/** The open item's place while loading: its first line, title and a few lines, holding still. */
function DetailSkeleton() {
  return (
    <div className="d-skel" aria-hidden>
      <div className="d-bar" />
      <div className="d-body">
        <div className="d-from"><span className="skeleton skeleton-c" style={{ width: 24, height: 24 }} /><span className="skeleton" style={{ width: 220, height: 10 }} /></div>
        <span className="skeleton" style={{ width: "72%", height: 30, marginTop: 18 }} />
        <span className="skeleton" style={{ width: "46%", height: 30, marginTop: 10 }} />
        {[92, 84, 88, 60].map((w, i) => <span key={i} className="skeleton" style={{ width: `${w}%`, height: 10, marginTop: i ? 12 : 32 }} />)}
      </div>
    </div>
  );
}

