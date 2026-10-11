import { useEffect, useRef, useState } from "react";
import { useShell } from "../../Shell";
import { firstName, timeOrDay } from "../../lib/format";
import { useOrg } from "../../lib/hooks";
import { openChat } from "../../lib/router";
import { OPEN_LIMIT, shownRows, type Placed, type Shelf } from "../../lib/shelves";
import { NEW_TASK_FOCUS } from "../../lib/threads";
import { Icon } from "../Icon";
import { Avatar, Popover } from "../ui";
import { openRow, showAllOpen, toggleShelf, useShelfUpkeep, useShelves } from "./useShelves";
import "./Shelves.css";

const MAC = typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.platform);
/** The modifier the jump keys use: ⌘ on a Mac, Ctrl elsewhere. */
export const JUMP_MOD = MAC ? "⌘" : "Ctrl";
const JUMPS = 9;

/** Every conversation across the office, on shelves by what it needs from you. `collapsed`: the 60px sidebar. */
export function Shelves({ collapsed }: { collapsed: boolean }) {
  const { shelves, error, view, visible, selected } = useShelves();
  const shell = useShell();
  useShelfUpkeep(shelves, selected);
  const open = (p: Placed) => (p.row.kind === "proposal" ? shell.reviewProposal(p.row.proposal) : openRow(p.row));
  const badges = useJumpKeys(visible, open);

  if (!shelves) return error ? <p className="shelves-note faint">{error}</p> : null;
  if (!shelves.length) return collapsed ? null : <p className="shelves-note faint">Conversations with your team show up here.</p>;
  const jump = new Map(visible.slice(0, JUMPS).map((p, i) => [p.row.key, i + 1]));
  return (
    <div className="shelves">
      {shelves.map((s) => (
        <ShelfSection
          key={s.id} shelf={s} rows={shownRows(s, view)} openAll={view.openAll} folded={view.collapsed.has(s.id)} sideCollapsed={collapsed}
          selected={selected} jump={badges ? jump : null} onOpen={open}
        />
      ))}
    </div>
  );
}

/** One shelf: its header (click to collapse), its rows, and for Open the rest behind "Show N more". */
function ShelfSection({ shelf, rows, openAll, folded, sideCollapsed, selected, jump, onOpen }: {
  shelf: Shelf;
  /** The rows it shows now. */
  rows: Placed[];
  /** Open shows every row. */
  openAll: boolean;
  /** You collapsed this shelf. */
  folded: boolean;
  sideCollapsed: boolean;
  selected: string | null;
  /** Each row's jump number while the modifier is held, else null. */
  jump: Map<string, number> | null;
  onOpen: (p: Placed) => void;
}) {
  const extra = shelf.id === "open" ? shelf.rows.length - OPEN_LIMIT : 0;
  return (
    <section className={`shelf shelf-${shelf.id}`} aria-label={shelf.label}>
      <button className="shelf-head" aria-expanded={!folded} onClick={() => toggleShelf(shelf.id)}>
        <Icon name="chevronRight" size={12} className="shelf-chev" />
        <span>{shelf.label}</span>
        <span className="shelf-count num">· {shelf.rows.length}</span>
      </button>
      {rows.map((p) => (
        <ShelfRow key={p.row.key} p={p} on={p.row.key === selected} sideCollapsed={sideCollapsed} jump={jump?.get(p.row.key) ?? null} onOpen={onOpen} />
      ))}
      {!folded && extra > 0 ? (
        <button className="shelf-more" onClick={() => showAllOpen(!openAll)}>{openAll ? "Show fewer" : `Show ${extra} more`}</button>
      ) : null}
    </section>
  );
}

/** What a row says on its right: when a snooze ends, that it waits for a free slot, or whose it is. An ongoing chat
 *  says "Chat" beside threads, and the employee's title on Team, where every row is one. */
function meta(p: Placed): string {
  if (p.shelf === "snoozed" && p.mark.snoozedUntil) return timeOrDay(p.mark.snoozedUntil);
  if (p.queued) return "queued";
  switch (p.row.kind) {
    case "thread": return firstName(p.row.employee.name);
    case "chat": return p.shelf === "team" ? p.row.employee.title : "Chat";
    case "proposal": return "Proposal";
  }
}

/** One conversation: avatar, title and meta; avatar only in the collapsed sidebar, with the rest in its tooltip. */
function ShelfRow({ p, on, sideCollapsed, jump, onOpen }: {
  p: Placed;
  /** It is the conversation open now. */
  on: boolean;
  sideCollapsed: boolean;
  /** Its jump number while the modifier is held. */
  jump: number | null;
  onOpen: (p: Placed) => void;
}) {
  const { row } = p;
  const who = row.kind === "proposal" ? { name: row.proposal.draft.name, hue: row.proposal.draft.hue } : row.employee;
  const tip = [row.title, p.reason].filter(Boolean).join(". ");
  const dot = p.shelf === "needsYou" ? "warn" : p.busy ? "live" : null;
  return (
    <button
      className={`side-item srow ${on ? "side-item-on" : ""}`} aria-current={on ? "true" : undefined} onClick={() => onOpen(p)}
      title={sideCollapsed ? undefined : tip} data-tip={sideCollapsed ? tip : undefined} aria-label={tip}
    >
      <span className="srow-av">
        <Avatar name={who.name} hue={who.hue} size={18} presence={p.queued ? "queued" : p.busy ? "working" : undefined} />
        {sideCollapsed && dot ? <span className={`side-dot side-dot-${dot}`} /> : null}
        {jump ? <span className="srow-jump num" aria-hidden>{jump}</span> : null}
      </span>
      <span className="side-text srow-title">{row.title}</span>
      <span className="srow-meta">{meta(p)}</span>
    </button>
  );
}

/** ⌘1 to ⌘9 (Ctrl elsewhere) open the first nine rows on screen, even while typing. Holding the modifier alone for
 *  200 ms shows their numbers; letting go, leaving the window or any other key hides them. */
function useJumpKeys(rows: Placed[], open: (p: Placed) => void): boolean {
  const [shown, setShown] = useState(false);
  const latest = useRef({ rows, open });
  latest.current = { rows, open };
  useEffect(() => {
    let timer: number | undefined;
    const hide = () => { window.clearTimeout(timer); setShown(false); };
    const onDown = (e: KeyboardEvent) => {
      if (e.isComposing) return;
      if (e.key === (MAC ? "Meta" : "Control")) {
        if (!e.repeat) { window.clearTimeout(timer); timer = window.setTimeout(() => setShown(true), 200); }
        return;
      }
      hide();
      const mod = MAC ? e.metaKey && !e.ctrlKey : e.ctrlKey && !e.metaKey;
      const digit = /^Digit([1-9])$/.exec(e.code);
      if (!mod || !digit || e.altKey || e.shiftKey) return;
      const p = latest.current.rows[Number(digit[1]) - 1];
      if (!p) return;
      e.preventDefault();
      latest.current.open(p);
    };
    window.addEventListener("keydown", onDown);
    window.addEventListener("keyup", hide);
    window.addEventListener("blur", hide);
    return () => { hide(); window.removeEventListener("keydown", onDown); window.removeEventListener("keyup", hide); window.removeEventListener("blur", hide); };
  }, []);
  return shown;
}

/** Starts a task with anyone on the team: pick them, and the new task opens. */
export function NewTaskButton({ collapsed }: { collapsed: boolean }) {
  const { snap } = useOrg();
  const shell = useShell();
  const ref = useRef<HTMLButtonElement>(null);
  const [open, setOpen] = useState(false);
  const employees = snap?.employees ?? [];
  return (
    <>
      <button ref={ref} className="side-search side-new" onClick={() => setOpen((o) => !o)} aria-expanded={open} aria-haspopup="menu" data-tip={collapsed ? "New task" : undefined} aria-label="New task">
        <Icon name="plus" size={15} />
        <span className="side-text">New task</span>
      </button>
      <Popover anchor={ref} open={open} onClose={() => setOpen(false)} align="start" width={240}>
        <div className="menu" role="menu" aria-label="New task for" data-esc-stop>
          {employees.length ? employees.map((e) => (
            <button key={e.id} type="button" role="menuitem" className="menu-item" onClick={() => { setOpen(false); openChat(e.id, NEW_TASK_FOCUS); }}>
              <Avatar name={e.name} hue={e.hue} size={18} />{e.name}
            </button>
          )) : (
            <button type="button" role="menuitem" className="menu-item" onClick={() => { setOpen(false); shell.hire(); }}>
              <Icon name="plus" size={13} />Hire your first employee
            </button>
          )}
        </div>
      </Popover>
    </>
  );
}
