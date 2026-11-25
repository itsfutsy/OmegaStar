import { Fragment, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { call, errorMessage } from "../api/client";
import type { BriefRow, Card, CardReply, EmployeeView, HireProposal, NewAttachment, OrgSnapshot, PersonView, Run, SkillInfo, Thread } from "../api/types";
import { askAboutCard, usePendingAnswers } from "../lib/cardAnswers";
import type { CardDecision, Decision, Group } from "../lib/decisions";
import { dateLong, firstName, listNames, plural, runKindLabel, shortAge, taskLabel, threadTitle, whenInline } from "../lib/format";
import { useThread } from "../lib/hooks";
import { lastMoved, useMarkSeen } from "../lib/marks";
import { openChat } from "../lib/router";
import { toolName } from "../lib/rules";
import { toast } from "../lib/toasts";
import { activityAt, archivable, didNotFinish, isLive, isOpen, isTeamWeek, latestFinished, rowThreads, STREAK, threadBusy, threadFocus, wasArchived, type OpenCard } from "../lib/threads";
import { kindNames } from "../lib/workTitle";
import { BlockedCard } from "../components/BlockedCard";
import { Icon, type IconName } from "../components/Icon";
import { Markdown } from "../components/Markdown";
import { ProposalActions, ProposalRecord, hireTitle } from "../components/ProposalCard";
import { Avatar, tipWhenCut } from "../components/ui";
import { CardImages } from "../components/conversation/CardImages";
import { CardLink, webUrl } from "../components/conversation/CardLink";
import { CardFile, CardList, CardProposal, CardText, CardView } from "../components/conversation/CardView";
import { lines } from "../components/conversation/RowHistory";
import { ThreadView, useRetry } from "../components/conversation/ThreadView";
import type { Item } from "../components/conversation/useStatuses";

// ---------------------------------------------------------------- the model: what each row is and needs

/** What an item needs from you, in one word. */
export type Word = "Hire" | "Decide" | "Approve" | "Do" | "Answer" | "Know" | "Reply" | "Failed";
export const WORDS: Word[] = ["Hire", "Decide", "Approve", "Do", "Answer", "Know", "Reply", "Failed"];

/** One row of the list: a hire proposal, a decision (with the group of one employee's cards it is in), a run holding
 *  cards you parked, or a line of the Archive under its day. */
export type Row =
  | { kind: "hire"; id: string; p: HireProposal }
  | { kind: "decision"; id: string; d: Decision<Item>; group: Group<Item> | null }
  | { kind: "parked"; id: string; run: Run }
  | { kind: "archived"; id: string; item: Item; day: string };

/** What an Inbox entry can do to the threads it is about. */
export type ItemActions = {
  /** Archive it. */
  done: () => void;
  /** Reply and archive it in one go. */
  sendDone: (text: string, attachments: NewAttachment[]) => Promise<void>;
  /** Answer the card at `index` of `run`'s cards. */
  answer: (run: Run, index: number, reply: CardReply) => Promise<void>;
  /** Answer these cards together once the Undo window has passed, settling the entries `ids`. */
  answerMany: (picks: { card: OpenCard; reply: CardReply }[], ids: string[]) => void;
};

const handOff = (r: Run): boolean => r.kind.type === "asked" || r.kind.type === "delegated" || r.kind.type === "answered";

/** A thread's own name: its title, a routine's label or a conversation's name. Null for a hand-off with no title of
 *  its own, which `threadTitle` names by how it came, as "Task from Bo". */
function ownName(r: Run, employees: EmployeeView[], people: PersonView[]): string | null {
  // A run you started by hand is named by what you asked, not "Manual run".
  if (r.kind.type === "manual" && !r.title) return taskLabel(r.prompt, r.attachments);
  const names = kindNames(r, employees, people);
  const title = threadTitle(r, names);
  return handOff(r) && title === runKindLabel(r.kind, names) ? null : title;
}

/** What an entry is about rather than how it came: the thread's own name, and on a hand-off who it is for (or, for an
 *  answer that came back, who it is from), as "Pricing page for Bo". */
function topic(t: Thread, employees: EmployeeView[], people: PersonView[]): string {
  const r = t.root;
  const names = kindNames(r, employees, people);
  const them = handOff(r) ? names.asker ?? names.manager : undefined;
  const other = them ? `${r.kind.type === "answered" ? "from" : "for"} ${them}` : null;
  return [ownName(r, employees, people), other].filter(Boolean).join(" ") || threadTitle(r, names);
}

/** The first line of what a run said, or why it said nothing. */
const firstLine = (r: Run): string => lines(r.summary)[0] ?? (didNotFinish(r) ? r.error ?? "Didn't finish" : "No report");

/** A list of first names, as "Ada, Bo and Cy". */
function names(ids: string[], employees: EmployeeView[]): string {
  const ns = [...new Set(ids)].flatMap((id) => employees.find((x) => x.id === id) ?? []).map((x) => firstName(x.name));
  return listNames(ns);
}

const NUMS = ["No", "One", "Two", "Three", "Four", "Five", "Six", "Seven", "Eight", "Nine"];
/** A count at the start of a sentence: "Two", and "12" past nine. */
const Count = (n: number) => NUMS[n] ?? String(n);

/** The word for what a decision needs. */
function wordOf(d: Decision<Item>): Word {
  switch (d.type) {
    case "card": {
      if (d.cards.length > 1) return "Answer";
      const a = d.cards[0].card.ask.type;
      return a === "choices" ? "Decide" : a === "items" ? "Approve" : a === "task" || a === "chore" ? "Do" : a === "question" ? "Answer" : "Know";
    }
    case "flag": return "Know";
    case "proposal": case "blocked": return "Approve";
    case "broken": case "failures": return "Failed";
    case "answered": return "Reply";
  }
}

export const rowWord = (r: Row): Word | null => (r.kind === "hire" ? "Hire" : r.kind === "decision" ? wordOf(r.d) : null);

/** A decision as the Inbox titles it, and one line on why. Where OmegaStar writes the title it asks the decision;
 *  a card's question, a flagged line and a reply are the employee's own words. */
function ask(d: Decision<Item>, who: string, employees: EmployeeView[], people: PersonView[]): { title: string; why: string | null } {
  const r = d.run;
  switch (d.type) {
    case "card": {
      if (d.cards.length === 1) return { title: d.cards[0].card.message, why: d.cards[0].card.why };
      // News asks nothing, so they do not wait for it.
      const answered = d.cards.filter((c) => c.card.answer).length;
      const asked = answered + d.cards.filter((c) => isOpen(c.card)).length;
      const root = d.thread.root;
      const named = root.title || root.kind.type === "scheduled" || root.kind.type === "task" ? ownName(root, employees, people) : null;
      const n = Count(d.cards.length);
      const title = named && named.length <= 44 && !/[.?!:]$/.test(named) ? `${named}: ${n.toLowerCase()} questions` : `${n} questions from ${who}`;
      return { title, why: asked > 1 ? `${answered} of ${asked} answered. ${who} hears your answers together, once all are in.` : null };
    }
    case "flag": return { title: d.text, why: null };
    case "proposal": return { title: `Approve ${who}'s skill changes?`, why: null };
    case "blocked":
      return {
        title: r.denials.length === 1 ? `Allow ${who} to use ${toolName(r.denials[0].tool)}?` : `Allow ${r.denials.length} actions ${who} was blocked on?`,
        why: r.denials.length === 1 ? r.denials[0].brief : null,
      };
    case "broken": {
      const name = ownName(d.thread.root, employees, people);
      return {
        title: d.times >= STREAK ? `${name ?? "A routine"} failed ${d.times} times in a row` : name ? `${name} didn't finish` : "Didn't finish",
        why: lines(r.error)[0] ?? null,
      };
    }
    case "failures": return { title: `${Count(d.failures.length)} runs didn't finish`, why: lines(r.error)[0] ?? null };
    case "answered": return { title: firstLine(r), why: null };
  }
}

/** How the open item's first line says what the employee did: "wants to hire", "asks you to decide". */
function verbOf(d: Decision<Item>): string {
  switch (d.type) {
    case "card": {
      if (d.cards.length > 1) return `has ${Count(d.cards.length).toLowerCase()} questions`;
      const a = d.cards[0].card.ask.type;
      return a === "choices" ? "asks you to decide" : a === "items" ? "asks you to approve" : a === "task" || a === "chore" ? "has a task for you" : a === "question" ? "asks" : "wants you to know";
    }
    case "flag": return "flagged this";
    case "proposal": return "proposes skill changes";
    case "blocked": return "was blocked";
    case "broken": case "failures": return "couldn't finish";
    case "answered": return "replied";
  }
}

/** When a decision started waiting on you. */
const since = (d: Decision<Item>) => d.run.endedAt ?? activityAt(d.item.thread);

/** "just now", "5 minutes ago", "3 hours ago", "2 days ago", else the date. */
function ageLong(iso: string | null): string {
  if (!iso) return "";
  const s = Math.max(0, (Date.now() - Date.parse(iso)) / 1000);
  const n = (v: number, unit: string) => `${v} ${unit}${v === 1 ? "" : "s"} ago`;
  if (s < 60) return "just now";
  if (s < 3600) return n(Math.floor(s / 60), "minute");
  if (s < 86_400) return n(Math.floor(s / 3600), "hour");
  if (s < 7 * 86_400) return n(Math.floor(s / 86_400), "day");
  return dateLong(iso);
}

export type RowContext = { employees: EmployeeView[]; people: PersonView[] };

/** What a row shows in the list: who, the title, the line under it, its word and its age. */
export type RowView = { title: string; meta: string; avatar: ReactNode; word: Word | null; side: string; sideTip: string; unread: boolean };

export function rowView(row: Row, { employees, people }: RowContext): RowView {
  if (row.kind === "hire") {
    const by = employees.find((e) => e.id === row.p.employeeId);
    return {
      title: hireTitle(row.p), meta: `${by ? firstName(by.name) : "An employee"} wants to hire`, word: "Hire",
      avatar: <Avatar name={row.p.draft.name} hue={row.p.draft.hue} size={28} proposed />,
      side: shortAge(row.p.createdAt), sideTip: dateLong(row.p.createdAt), unread: false,
    };
  }
  if (row.kind === "parked") {
    const e = employees.find((x) => x.id === row.run.employeeId);
    const cards = row.run.cards.filter((c) => c.parked);
    const at = row.run.endedAt ?? row.run.createdAt;
    return {
      title: cards.length === 1 ? cards[0].message : `${Count(cards.length)} parked questions`,
      meta: `${e ? firstName(e.name) : "Former employee"} · ${topic({ root: row.run, replies: [], related: [] }, employees, people)}`,
      avatar: e ? <Avatar name={e.name} hue={e.hue} image={e.image} size={28} /> : <span className="irow-noav" />,
      word: null, side: shortAge(at), sideTip: dateLong(at), unread: false,
    };
  }
  if (row.kind === "archived") {
    const { thread: t, employee: e } = row.item;
    const team = isTeamWeek(t);
    const run = latestFinished(t);
    const text = team ? names(rowThreads(t).map((x) => x.root.employeeId), employees) : run.cards[0] && !lines(run.summary).length ? run.cards[0].message : firstLine(run);
    return {
      title: text, meta: team ? `Your team's week · ${rowThreads(t).length} reviews` : `${firstName(e.name)} · ${topic(t, employees, people)}`,
      avatar: <Avatar name={e.name} hue={e.hue} image={e.image} size={28} />, word: didNotFinish(run) ? "Failed" : null,
      side: "", sideTip: dateLong(activityAt(t)), unread: false,
    };
  }
  const d = row.d;
  const e = employees.find((x) => x.id === d.run.employeeId) ?? d.item.employee;
  const who = firstName(e.name);
  const { title } = ask(d, who, employees, people);
  const meta = d.type === "failures" ? names(d.failures.map((f) => f.run.employeeId), employees) : `${who} · ${topic(d.thread, employees, people)}`;
  const snoozed = d.item.status.snoozedUntil;
  return {
    title, meta, avatar: <Avatar name={e.name} hue={e.hue} image={e.image} size={28} />, word: wordOf(d),
    side: snoozed ? `Back ${whenInline(snoozed).replace(/^(today|tomorrow) /, "")}` : shortAge(since(d)),
    sideTip: snoozed ? `Comes back ${whenInline(snoozed)}` : dateLong(since(d)), unread: d.item.status.unread,
  };
}

// ---------------------------------------------------------------- a row in the list

/** One row of the list: avatar, the title (two lines at most), who and what about, and its word and age in a fixed
 *  right track. It only selects. `held`: it left while the pointer was over the list, so it keeps its place, faded. */
export function InboxRow({ id, view, selected, held, onSelect }: { id: string; view: RowView; selected: boolean; held: boolean; onSelect: () => void }) {
  return (
    <div
      role="option" id={`irow-${id}`} aria-selected={selected} tabIndex={selected ? 0 : -1} aria-disabled={held || undefined} data-row={id}
      className={`irow ${selected ? "irow-on" : ""} ${view.unread ? "irow-unread" : ""} ${held ? "irow-held" : ""}`}
      onClick={held ? undefined : onSelect}
    >
      {view.avatar}
      <span className="item-title irow-t" onPointerEnter={tipWhenCut}>{view.title}</span>
      <span className="irow-m">{view.meta}</span>
      <span className="irow-r">
        {held ? <span className="irow-cleared"><Icon name="check" size={12} /> Cleared</span> : (
          <>
            {view.word ? <span className={`tag ${view.word === "Hire" ? "tag-hire" : view.word === "Failed" ? "tag-attn" : ""}`}>{view.word}</span> : null}
            {view.side ? <span className="irow-age num" title={view.sideTip}>{view.side}</span> : null}
          </>
        )}
      </span>
    </div>
  );
}

/** A row's place while loading: the same geometry, holding still. */
export function RowSkeleton({ w }: { w: number }) {
  return (
    <div className="irow irow-skel" aria-hidden>
      <span className="skeleton skeleton-c" style={{ width: 28, height: 28 }} />
      <span className="irow-t"><span className="skeleton" style={{ width: `${w}%`, height: 10 }} /></span>
      <span className="irow-m"><span className="skeleton" style={{ width: `${w - 24}%`, height: 8 }} /></span>
      <span className="irow-r"><span className="skeleton" style={{ width: 44, height: 20 }} /></span>
    </div>
  );
}

// ---------------------------------------------------------------- the open item

/** What the open item needs from the screen. */
export type DetailContext = RowContext & {
  snap: OrgSnapshot | null;
  catalog: SkillInfo[] | undefined;
  actionsFor: (d: Decision<Item>) => ItemActions;
  /** Answer a parked card. */
  answerParked: (run: Run, index: number, reply: CardReply) => Promise<void>;
  /** Settle entries `ids` while `go` runs; they come back if it resolves false. */
  settle: (ids: string[], go: () => Promise<boolean>) => void;
  /** Every other row from this employee, for a hire proposal's "Also waiting from". */
  alsoFrom: (employeeId: string, except: string) => { id: string; title: string; word: Word | null; age: string }[];
  select: (id: string) => void;
  reviewProposal: (p: HireProposal, from: HTMLElement | null) => void;
  reopen: (id: string) => void;
};

type Portal = (node: ReactNode) => ReactNode;

/** The open item's body, and through `foot` (a portal) its actions in the bar at the foot. `slot`: the bar's place for
 *  the open card's own buttons (CardView's actionsSlot), so the answer sits where Enter presses it. */
export function DetailBody({ row, ctx, foot, slot }: { row: Row; ctx: DetailContext; foot: HTMLElement | null; slot: HTMLElement | null }) {
  const actions: Portal = (node) => (foot ? createPortal(node, foot) : null);
  if (row.kind === "hire") return <HireBody p={row.p} ctx={ctx} actions={actions} />;
  if (row.kind === "parked") return <ParkedBody run={row.run} ctx={ctx} slot={slot} />;
  if (row.kind === "archived") return <ArchivedBody row={row} ctx={ctx} actions={actions} />;
  return <DecisionBody d={row.d} group={row.group} ctx={ctx} actions={actions} slot={slot} />;
}

/** The open item's first line: who, what they did, what about and when, on one line with even space round each dot.
 *  When what it is about doesn't fit whole, it is left out (the list's row says it), never cut short. */
function From({ e, verb, about, at }: { e: { name: string; hue: number; image?: string | null } | null; verb: ReactNode; about: string | null; at: string | null }) {
  const line = useRef<HTMLDivElement>(null);
  const [fits, setFits] = useState(true);
  useLayoutEffect(() => {
    const el = line.current;
    if (!el || !about) return;
    const check = () => {
      // Measure with it shown; keep it only if the whole line fits.
      el.dataset.measure = "";
      const over = el.scrollWidth > el.clientWidth + 1;
      delete el.dataset.measure;
      setFits(!over);
    };
    check();
    const ro = new ResizeObserver(check);
    ro.observe(el);
    return () => ro.disconnect();
  }, [about]);
  const shown = about && fits;
  return (
    <div ref={line} className="d-from">
      {e ? <Avatar name={e.name} hue={e.hue} image={e.image} size={24} /> : null}
      <span className="d-who">{e ? <><b>{e.name}</b> </> : null}{verb}</span>
      {about ? <><span className={`d-sep ${shown ? "" : "d-off"}`} aria-hidden>·</span><span className={`d-about ${shown ? "" : "d-off"}`}>{about}</span></> : null}
      {at ? <><span className="d-sep" aria-hidden>·</span><span className="d-at" title={dateLong(at)}>{ageLong(at)}</span></> : null}
    </div>
  );
}

/** The open item's title: Display, a step smaller when it is a long question in the employee's own words, or over a
 *  card's brief. */
const Title = ({ text, memo = false }: { text: string; memo?: boolean }) => <h2 className={`d-title ${text.length > 90 ? "d-title-sm" : memo ? "d-title-memo" : ""}`}>{text}</h2>;

/** A card's brief: each fact once, in labelled rows, with the attachment a row shows under its text. */
function Brief({ card, runId, index }: { card: Card; runId: string; index: number }) {
  const link = webUrl(card.link);
  const shown = (shows: BriefRow["shows"]): ReactNode => {
    switch (shows) {
      case "images": return card.images.length ? <div className="d-brief-pics"><CardImages paths={card.images.slice(0, 3)} /></div> : null;
      case "file": return card.file ? <div className="cv-card d-brief-att"><CardFile runId={runId} index={index} path={card.file} /></div> : null;
      case "link": return link ? <div className="cv-card d-brief-att"><div className="card-about"><CardLink url={link} /></div></div> : null;
      default: return null;
    }
  };
  return (
    <dl className="d-brief">
      {card.brief.map((r, i) => (
        <Fragment key={i}>
          <dt>{r.label}</dt>
          <dd><Markdown text={r.text} breaks />{shown(r.shows)}</dd>
        </Fragment>
      ))}
    </dl>
  );
}

/** A card as the card under its brief shows it: without the attachments its brief's rows show already. */
function unshown(card: Card): Card {
  const shows = new Set(card.brief.map((r) => r.shows));
  return { ...card, images: shows.has("images") ? [] : card.images, file: shows.has("file") ? null : card.file, link: shows.has("link") ? null : card.link };
}

/** The report a decision came from, in full: the evidence for it. */
function Report({ who, text }: { who: string; text: string | null }) {
  if (!text?.trim()) return null;
  return (
    <section className="d-sec d-report" aria-label={`${who}'s report`}>
      <h3 className="d-sec-h">From {who}'s report</h3>
      <div className="d-report-body"><Markdown text={text} /></div>
    </section>
  );
}

/** The main action's button for the action bar: black, with ↵ on it. */
function Main({ label, icon, disabled, title, onClick }: { label: string; icon?: IconName; disabled?: boolean; title?: string; onClick: () => void }) {
  return (
    <button type="button" className="btn btn-primary" data-main disabled={disabled} title={title} onClick={onClick}>
      {icon ? <Icon name={icon} size={14} /> : null}{label}<span className="kc">↵</span>
    </button>
  );
}

function HireBody({ p, ctx, actions }: { p: HireProposal; ctx: DetailContext; actions: Portal }) {
  const proposer = ctx.employees.find((e) => e.id === p.employeeId);
  const who = proposer ? firstName(proposer.name) : "them";
  const also = ctx.alsoFrom(p.employeeId, p.id);
  return (
    <>
      <From e={proposer ?? null} verb="wants to hire" about={null} at={p.createdAt} />
      <h2 className="d-title">{hireTitle(p)}</h2>
      <ProposalRecord p={p} proposer={proposer} snap={ctx.snap} catalog={ctx.catalog} />
      {also.length ? (
        <section className="d-sec d-also" aria-label={`Also waiting from ${who}`}>
          <h3 className="d-sec-h">Also waiting from {who} <span className="d-sec-n num">{also.length}</span></h3>
          <ul className="d-also-list">
            {also.map((a) => (
              <li key={a.id}>
                <button type="button" className="d-also-row" onClick={() => ctx.select(a.id)}>
                  <span className="d-also-t">{a.title}</span>
                  {a.word ? <span className={`tag ${a.word === "Failed" ? "tag-attn" : ""}`}>{a.word}</span> : null}
                  <span className="d-also-age num">{a.age}</span>
                </button>
              </li>
            ))}
          </ul>
        </section>
      ) : null}
      {actions(
        <ProposalActions p={p} proposer={proposer} onReview={(from) => ctx.reviewProposal(p, from)} onDecline={(go) => ctx.settle([p.id], go)} />,
      )}
    </>
  );
}

function DecisionBody({ d, group, ctx, actions, slot }: { d: Decision<Item>; group: Group<Item> | null; ctx: DetailContext; actions: Portal; slot: HTMLElement | null }) {
  const { employees, people } = ctx;
  const e = employees.find((x) => x.id === d.run.employeeId) ?? d.item.employee;
  const who = firstName(e.name);
  const act = ctx.actionsFor(d);
  const { title, why } = ask(d, who, employees, people);
  const busy = threadBusy(d.thread);
  const root = d.thread.root;
  const failures = d.type === "failures" ? d.failures : null;
  const also = d.type === "card" ? names(d.also.map((r) => r.employeeId).filter((id) => id !== e.id), employees) : "";
  const canArchive = failures ? failures.some((f) => archivable(f.thread, f.item.thread)) : archivable(d.thread, d.item.thread);
  const archiveTip = busy ? `${who} is still working on it` : "Move it to the Archive (E)";
  const archive = (main: boolean, label = "Archive") => main
    ? <Main label={label} icon="archive" disabled={!canArchive} title={archiveTip} onClick={act.done} />
    : <button type="button" className="btn" disabled={!canArchive} title={archiveTip} onClick={act.done}><Icon name="archive" size={14} /> {label}</button>;

  let body: ReactNode = null;
  let foot: ReactNode = null;
  switch (d.type) {
    case "card":
      if (d.cards.length > 1) {
        body = <CardSet d={d} e={e} act={act} actions={actions} />;
      } else {
        const c = d.cards[0];
        body = (
          <>
            {c.card.brief.length ? <Brief card={c.card} runId={c.run.id} index={c.index} /> : null}
            <div className="d-card d-card-main"><CardView card={unshown(c.card)} runId={c.run.id} index={c.index} e={e} working={isLive(c.run)} onAnswer={(reply) => act.answer(c.run, c.index, reply)} actionsSlot={slot} /></div>
            {also ? <p className="d-note">{also} {d.also.length === 1 ? "asks" : "ask"} the same. Your answer goes to everyone asking.</p> : null}
            {d.differ.length ? <OtherChoices cards={d.differ} employees={employees} act={act} /> : null}
            {group ? <GroupMore d={d} group={group} who={who} act={act} actions={actions} /> : null}
          </>
        );
      }
      break;
    case "blocked":
      body = <div className="d-card d-card-main"><BlockedCard run={d.run} employee={e} /></div>;
      break;
    case "flag":
      foot = archive(true);
      break;
    case "proposal":
      // It waits in its thread, where a reply can correct it before you approve.
      body = <ItemThread item={d.item} e={e} onSendDone={act.sendDone} />;
      break;
    case "broken": {
      const retry = root.kind.type !== "scheduled";
      const more = lines(d.run.error).slice(1);
      body = more.length ? <pre className="d-error">{more.join("\n")}</pre> : null;
      foot = retry ? <><RetryMain run={d.run} who={who} disabled={busy} />{archive(false)}</> : archive(true);
      break;
    }
    case "failures":
      body = <FailureList failures={d.failures} employees={employees} people={people} />;
      foot = <><RetryAllMain d={d} />{archive(false, "Archive all")}</>;
      break;
    case "answered":
      body = <ItemThread item={d.item} e={e} onSendDone={act.sendDone} />;
      foot = archive(true);
      break;
  }
  // A card's brief holds what the report would say, so the report shows only beside cards without one.
  const open = d.type === "card" ? d.cards.filter((c) => isOpen(c.card)) : [];
  const briefed = open.length > 0 && open.every((c) => c.card.brief.length > 0);
  const memo = d.type === "card" && d.cards.length === 1 && d.cards[0].card.brief.length > 0;
  const report = d.type === "answered" || d.type === "proposal" || failures || briefed ? null : d.run.summary;
  return (
    <>
      {failures
        ? <From e={null} verb={<><b>{names(failures.map((f) => f.run.employeeId), employees)}</b> couldn't finish</>} about={null} at={since(d)} />
        : <From e={e} verb={verbOf(d)} about={d.type === "answered" ? null : topic(d.thread, employees, people)} at={since(d)} />}
      <Title text={d.type === "answered" ? topic(d.thread, employees, people) : title} memo={memo} />
      {why && !memo ? <p className="d-why">{why}</p> : null}
      {body}
      <Report who={who} text={report} />
      {foot ? actions(foot) : null}
    </>
  );
}

/** Retry a run that didn't finish, from where it stopped. It stays in the Inbox until the retry is through. */
function RetryMain({ run, who, disabled }: { run: Run; who: string; disabled: boolean }) {
  const { busy, retry } = useRetry(run, who);
  return <Main label={busy ? "Trying again" : "Retry"} icon="refresh" disabled={disabled || busy} title="Try it again from where it stopped" onClick={() => void retry()} />;
}

/** Retry all, for runs that didn't finish on the same error. Each retry carries on where it stopped. */
function RetryAllMain({ d }: { d: Extract<Decision<Item>, { type: "failures" }> }) {
  const [trying, setTrying] = useState(false);
  const retry = async () => {
    setTrying(true);
    const tries = await Promise.allSettled(d.failures.map((f) => call("retry_run", { runId: f.run.id })));
    // An answer OmegaStar already sent again is on its way: nothing to retry, and nothing went wrong.
    const again = (t: PromiseSettledResult<unknown>) => t.status === "rejected" && /already went again/.test(errorMessage(t.reason));
    const went = tries.filter((t) => t.status === "fulfilled" || again(t)).length;
    const failed = tries.find((t) => t.status === "rejected" && !again(t));
    if (went) toast(`Trying ${plural(went, "run")} again`, "ok");
    if (failed?.status === "rejected") toast(errorMessage(failed.reason), "err");
    if (!went) setTrying(false);
  };
  return <Main label={trying ? "Trying again" : "Retry all"} icon="refresh" disabled={trying} title="Try each one again from where it stopped" onClick={() => void retry()} />;
}

/** What a card's employee recommends, in a few words: the choice they marked, or how many of a list they ticked. */
function recommends(card: Card): string | null {
  const a = card.ask;
  if (a.type === "choices") return a.choices[a.recommended] ?? null;
  if (a.type !== "items") return null;
  const n = a.items.filter((i) => i.recommended).length;
  return n ? `${a.verb} ${n} of ${a.items.length}` : null;
}

/**
 * A run's several questions, one under another. A choice shows its choices, and No, with the employee's pick already
 * chosen; change any, then one Send answers them all, after an Undo window. Reply in words answers a choice in your own words
 * instead, or asks about it first. A question that needs words, a list or a to-do is answered on its own card, and Send
 * waits for it. While the run is still working nothing is answered yet.
 */
function CardSet({ d, e, act, actions }: { d: CardDecision<Item>; e: EmployeeView; act: ItemActions; actions: Portal }) {
  const who = firstName(e.name);
  const going = usePendingAnswers();
  const on = (c: OpenCard) => going.some((p) => p.runId === c.run.id && p.index === c.index);
  const choices = d.cards.filter((c) => c.card.ask.type === "choices" && isOpen(c.card) && !on(c));
  const [picks, setPicks] = useState<Record<number, number | "no">>(() =>
    Object.fromEntries(choices.map((c) => [c.index, c.card.ask.type === "choices" ? c.card.ask.recommended : 0])));
  const working = d.cards.some((c) => isLive(c.run));
  // Cards answered on their own (words, a list, a to-do) must be in before Send.
  const waiting = d.cards.filter((c) => c.card.ask.type !== "choices" && c.card.ask.type !== "news" && isOpen(c.card) && !on(c));
  // A choice you would rather answer in your own words, or ask about first: its line, open under its choices, and the
  // words on it, kept here so Undo gives them back.
  const [writing, setWriting] = useState<Record<number, boolean>>({});
  const [texts, setTexts] = useState<Record<number, string>>({});
  const [asking, setAsking] = useState(false);
  const askAbout = (c: OpenCard) => async (text: string) => {
    setAsking(true);
    try { return await askAboutCard(c.run.id, c.index, who, text); } finally { setAsking(false); }
  };
  const n = choices.length;
  const reply = (pick: number | "no" = 0): CardReply => (pick === "no" ? { type: "reject" } : { type: "choice", index: pick });
  const send = () => act.answerMany(choices.map((c) => ({ card: c, reply: reply(picks[c.index]) })), [d.id]);
  const label = n === 2 ? "Send both answers" : n === 1 ? "Send the answer" : `Send ${n} answers`;
  return (
    <>
      <ol className="d-qs">
        {d.cards.map((c) => {
          const a = c.card.ask;
          return (
            <li key={c.index} className="d-q">
              <div className="d-q-t">{c.card.message}</div>
              {c.card.brief.length ? <Brief card={c.card} runId={c.run.id} index={c.index} /> : c.card.why ? <div className="d-q-why">{c.card.why}</div> : null}
              {a.type === "choices" && isOpen(c.card) && !on(c) ? (<>
                <CardProposal proposal={c.card.proposal} />
                <div className="d-q-picks">
                  <div className="d-q-radios" role="radiogroup" aria-label={c.card.message}>
                    {[...a.choices.map((label, i) => [i, label] as const), ["no", "No"] as const].map(([pick, label]) => (
                      <button
                        key={pick} type="button" className="chip" role="radio" aria-checked={picks[c.index] === pick} aria-pressed={picks[c.index] === pick}
                        disabled={working} onClick={() => setPicks((p) => ({ ...p, [c.index]: pick }))}
                      >
                        {label}
                      </button>
                    ))}
                  </div>
                  <button
                    type="button" className="btn btn-ghost btn-sm" aria-expanded={!!writing[c.index]} disabled={working}
                    onClick={() => setWriting((w) => ({ ...w, [c.index]: !w[c.index] }))}
                  >
                    <Icon name="reply" size={14} /> Reply in words
                  </button>
                  <span className="d-q-pick">{who}'s pick: {a.choices[a.recommended]}</span>
                </div>
                {writing[c.index] ? (
                  <div className="d-q-words enter-drop">
                    <CardText
                      name={who} hint={`Your answer for ${who}, or a question about it`} autoFocus sending={working || asking}
                      text={texts[c.index] ?? ""} setText={(t) => setTexts((x) => ({ ...x, [c.index]: t }))}
                      onText={(t) => void act.answer(c.run, c.index, { type: "text", text: t })} onAsk={askAbout(c)}
                    />
                  </div>
                ) : null}
                <CardContext card={unshown(c.card)} runId={c.run.id} index={c.index} />
              </>) : (
                <div className="d-card"><CardView card={unshown(c.card)} runId={c.run.id} index={c.index} e={e} working={isLive(c.run)} onAnswer={(reply) => act.answer(c.run, c.index, reply)} /></div>
              )}
            </li>
          );
        })}
      </ol>
      {n ? actions(
        <Main
          label={label} icon="send" disabled={working || waiting.length > 0}
          title={working ? `Waiting for ${who} to finish` : waiting.length ? `Answer ${who}'s other ${waiting.length === 1 ? "question" : "questions"} first` : undefined}
          onClick={send}
        />,
      ) : null}
    </>
  );
}

/** What a choice card is about, above its choices: its link, up to three images and the file it is about (a draft),
 *  as the card shows them in a conversation. */
function CardContext({ card, runId, index }: { card: Card; runId: string; index: number }) {
  const link = webUrl(card.link);
  if (!link && !card.images.length && !card.file) return null;
  return (
    <div className="d-q-about">
      {link || card.images.length ? (
        <div className="card-about">
          {link ? <CardLink url={link} /> : null}
          {card.images.length ? <CardImages paths={card.images.slice(0, 3)} /> : null}
        </div>
      ) : null}
      {card.file ? <CardFile runId={runId} index={index} path={card.file} /> : null}
    </div>
  );
}

/** The others in a group: one employee's cards about the same thing. Each shows its pick, and one button answers the
 *  whole group with them. */
function GroupMore({ d, group, who, act, actions }: { d: CardDecision<Item>; group: Group<Item>; who: string; act: ItemActions; actions: Portal }) {
  const going = usePendingAnswers();
  const others = group.decisions.filter((x) => x.id !== d.id);
  if (!others.length) return null;
  const ready = group.decisions.filter((x) => {
    const c = x.cards[0];
    return c.card.ask.type === "choices" && isOpen(c.card) && !isLive(c.run) && !going.some((p) => p.runId === c.run.id && p.index === c.index);
  });
  const recommended = (c: OpenCard): CardReply => ({ type: "choice", index: c.card.ask.type === "choices" ? c.card.ask.recommended : 0 });
  const about = group.about.replace(/[-_]+/g, " ");
  return (
    <>
      <section className="d-sec" aria-label={`${others.length} more from ${who} about ${about}`}>
        <h3 className="d-sec-h">{Count(others.length)} more from {who} about {about}</h3>
        <ul className="d-also-list">
          {others.map((x) => (
            <li key={x.id} className="d-also-row d-also-still">
              <span className="d-also-t">{x.cards[0].card.message}</span>
              <span className="d-also-pick">{recommends(x.cards[0].card)}</span>
            </li>
          ))}
        </ul>
      </section>
      {ready.length > 1 ? actions(
        <button type="button" className="btn" onClick={() => act.answerMany(ready.map((x) => ({ card: x.cards[0], reply: recommended(x.cards[0]) })), ready.map((x) => x.id))}>
          <Icon name="checklist" size={14} /> Answer {ready.length === 2 ? "both" : `all ${ready.length}`} with {who}'s picks
        </button>,
      ) : null}
    </>
  );
}

/** Others asking the same thing with choices of their own: each one's question and recommendation, answered here too.
 *  Whichever you answer goes to everyone asking. */
function OtherChoices({ cards, employees, act }: { cards: OpenCard[]; employees: EmployeeView[]; act: ItemActions }) {
  return (
    <section className="d-sec" aria-label="Others asking the same">
      {cards.map((c) => {
        const e = employees.find((x) => x.id === c.run.employeeId);
        if (!e) return null;
        return (
          <div key={`${c.run.id}:${c.index}`} className="d-other">
            <h3 className="d-sec-h">{firstName(e.name)} asks the same{c.card.ask.type === "choices" ? ", with other choices" : ""}</h3>
            <p className="d-q-t">{c.card.message}</p>
            {c.card.why ? <p className="d-q-why">{c.card.why}</p> : null}
            <div className="d-card"><CardView card={c.card} runId={c.run.id} index={c.index} e={e} working={isLive(c.run)} onAnswer={(reply) => act.answer(c.run, c.index, reply)} /></div>
          </div>
        );
      })}
      <p className="d-note">Whichever you answer goes to everyone asking.</p>
    </section>
  );
}

/** The runs an entry of failures stands for: who, what each was about, when it stopped, and the way to its thread. */
function FailureList({ failures, employees, people }: { failures: Extract<Decision<Item>, { type: "failures" }>["failures"]; employees: EmployeeView[]; people: PersonView[] }) {
  return (
    <ul className="d-also-list d-fails">
      {failures.map((f) => {
        const e = employees.find((x) => x.id === f.run.employeeId);
        const at = f.run.endedAt ?? f.run.createdAt;
        return (
          <li key={f.id}>
            <button type="button" className="d-also-row" disabled={!e} title={e ? "Open the conversation" : undefined} onClick={() => e && openChat(e.id, threadFocus(f.thread.root.id))}>
              {e ? <Avatar name={e.name} hue={e.hue} image={e.image} size={20} /> : <span className="irow-noav" />}
              <span className="d-also-t"><b>{e ? firstName(e.name) : "Former employee"}</b> <span className="muted">· {topic(f.thread, employees, people)}</span></span>
              <span className="d-also-age num" title={dateLong(at)}>{shortAge(at)}</span>
            </button>
          </li>
        );
      })}
    </ul>
  );
}

/** A run holding cards you parked: its cards, answered here as on any card. */
function ParkedBody({ run, ctx, slot }: { run: Run; ctx: DetailContext; slot: HTMLElement | null }) {
  const e = ctx.employees.find((x) => x.id === run.employeeId);
  if (!e) return null;
  const who = firstName(e.name);
  const cards = run.cards.flatMap((card, index) => (card.parked ? [{ card, index }] : []));
  const one = cards.length === 1 ? cards[0] : null;
  const working = isLive(run);
  const answer = (index: number, reply: CardReply) => ctx.answerParked(run, index, reply);
  const memo = !!one?.card.brief.length;
  return (
    <>
      <From e={e} verb="asked, and you parked it" about={topic({ root: run, replies: [], related: [] }, ctx.employees, ctx.people)} at={run.endedAt ?? run.createdAt} />
      <Title text={one ? one.card.message : `${Count(cards.length)} parked questions`} memo={memo} />
      {one?.card.why && !memo ? <p className="d-why">{one.card.why}</p> : null}
      {one && memo ? <Brief card={one.card} runId={run.id} index={one.index} /> : null}
      <div className="d-card d-card-main">
        {one
          ? <CardView card={unshown(one.card)} runId={run.id} index={one.index} e={e} working={working} onAnswer={(reply) => answer(one.index, reply)} actionsSlot={slot} />
          : <CardList runId={run.id} cards={cards} e={e} working={working} onAnswer={answer} />}
      </div>
      <Report who={who} text={memo ? null : run.summary} />
    </>
  );
}

/** A line of the Archive, opened: its whole thread, and Reopen when you archived something in it. */
function ArchivedBody({ row, ctx, actions }: { row: Extract<Row, { kind: "archived" }>; ctx: DetailContext; actions: Portal }) {
  const { thread: t, employee: e } = row.item;
  const team = isTeamWeek(t);
  const view = rowView(row, ctx);
  return (
    <>
      {team
        ? <From e={null} verb={<b>{view.title}</b>} about={null} at={activityAt(t)} />
        : <From e={e} verb="reported" about={null} at={activityAt(t)} />}
      {/* Named by what it was about; the report under it says what it said, once. */}
      <Title text={team ? "Your team's week" : topic(t, ctx.employees, ctx.people)} />
      <ItemThread item={row.item} e={e} onSendDone={null} />
      {wasArchived(t) ? actions(<Main label="Reopen" icon="undo" title="Bring it back to the Inbox (E)" onClick={() => ctx.reopen(t.root.id)} />) : null}
    </>
  );
}

/** An open entry's thread, live: the same conversation the conversation screen shows, seen once it is in front of you.
 *  The row's other runs head it. */
function ItemThread({ item, e, onSendDone }: { item: Item; e: EmployeeView; onSendDone: ItemActions["sendDone"] | null }) {
  const id = item.thread.root.id;
  const live = useThread(id);
  useMarkSeen(item.status.state === "done" ? null : id, lastMoved(live.turns?.map((t) => t.run) ?? []));
  return (
    <div className="d-thread">
      <ThreadView e={e} rootId={id} thread={live} row={item.thread} onSendDone={item.status.state === "done" ? null : onSendDone} autoFocus={false} recent />
    </div>
  );
}
