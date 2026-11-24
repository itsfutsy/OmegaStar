import { useRef, useState } from "react";
import { call, errorMessage } from "../api/client";
import type { AccountUsage, UsageLimits, UsageWindow } from "../api/types";
import { clockHM, relTime, resetsLabel, untilShort } from "../lib/format";
import { useNow, useQuery, useUsage } from "../lib/hooks";
import { toast } from "../lib/toasts";
import { Icon } from "./Icon";
import { Avatar, Popover } from "./ui";
import "./UsageCard.css";

const level = (w: UsageWindow | null) => (!w ? "" : w.percent >= 95 ? "usage-err" : w.percent >= 80 ? "usage-warn" : "");
/** A window's percent; empty until it is known, so nothing stands in for a number. */
const pct = (w: UsageWindow | null) => (w ? `${Math.round(w.percent)}%` : "");
const said = (w: UsageWindow | null, known: boolean) => (w ? `${pct(w)} used` : known ? "not reported" : "not checked yet");

/** One window: label, meter, percent. The tooltip names the card and holds the reset time and when it was checked; the
 *  reset time is on screen once the window runs high. */
function Window({ label, w, now, known, checked, near, other }: {
  label: string; w: UsageWindow | null; now: number; known: boolean; checked: string;
  /** The limit you will reach first: on a short window it may be the only row, so its tooltip names the other too. */
  near: boolean; other: string;
}) {
  const resets = w ? resetsLabel(w, new Date(now)) : "";
  const lv = level(w);
  const tip = `Claude usage, ${label}: ${said(w, known)}${w && resets ? ` · ${resets}` : ""}${near ? ` · ${other}` : ""} · ${checked}`;
  return (
    <span className={`usage-row ${lv} ${near ? "usage-near" : ""}`} data-tip={tip}>
      <span className="usage-name">{label}</span>
      <span className="usage-bar"><span style={{ transform: `scaleX(${w ? Math.min(100, w.percent) / 100 : 0})` }} /></span>
      <span className="mono num">{pct(w)}</span>
      {/* Until the first reading arrives the row is its meter and nothing else: no placeholder to flash. */}
      {lv && resets ? <span className="usage-sub">{resets}</span> : !w && known ? <span className="usage-sub">Not reported</span> : null}
    </span>
  );
}

/** A limit as the detail shows it: its windows over all accounts, and how to read one account's. */
type Limit = { label: string; total: UsageWindow; each: (a: AccountUsage) => UsageWindow | null };

/** Windows averaged as the CLI proxy's pool is: full only when every account is, resetting when the first does. */
function average(ws: (UsageWindow | null)[]): UsageWindow | null {
  const seen = ws.filter((w): w is UsageWindow => !!w);
  if (!seen.length) return null;
  const resets = seen.map((w) => w.resetsAt).filter((t): t is string => !!t).sort((a, b) => Date.parse(a) - Date.parse(b));
  return { percent: seen.reduce((n, w) => n + w.percent, 0) / seen.length, resetsAt: resets[0] ?? null, resetsText: null };
}

/** The 5-hour and 7-day limits, then each weekly limit on one model that any account reports. */
function limits(usage: UsageLimits): Limit[] {
  const models = [...new Set(usage.accounts.flatMap((a) => a.modelWeeks.map((m) => m.model)))];
  const all: { label: string; total: UsageWindow | null; each: Limit["each"] }[] = [
    { label: "5-hour", total: usage.fiveHour, each: (a) => a.fiveHour },
    { label: "7-day", total: usage.sevenDay, each: (a) => a.sevenDay },
    ...models.map((m) => {
      const each = (a: AccountUsage) => a.modelWeeks.find((x) => x.model === m)?.window ?? null;
      return { label: `7-day · ${m}`, total: average(usage.accounts.map(each)), each };
    }),
  ];
  return all.filter((l): l is Limit => !!l.total);
}

/** A steady colour per account name, as employees have. */
const hueOf = (name: string) => [...name].reduce((h, c) => (h * 31 + c.charCodeAt(0)) % 360, 7);
const hot = (w: UsageWindow) => (w.percent >= 80 ? " ud-hot" : "");

/** One account's share of a limit: its meter fills the tile behind its name, with time left until it resets. */
function Tile({ a, w, now }: { a: AccountUsage; w: UsageWindow; now: number }) {
  const left = w.resetsAt ? untilShort(w.resetsAt, now) : "";
  return (
    <div className={`ud-tile${hot(w)}`} data-tip={`${a.name}: ${pct(w)} used${w.resetsAt ? ` · ${resetsLabel(w, new Date(now))}` : ""}`}>
      <span className="ud-fill" style={{ width: `${Math.min(100, w.percent)}%` }} />
      <Avatar name={a.name} hue={hueOf(a.name)} size={16} />
      <span className="ud-name">{a.name.split("@")[0]}</span>
      <span className="ud-pct num">{pct(w)}</span>
      {left ? <span className="ud-left num"><Icon name="refresh" size={10} />{left}</span> : null}
    </div>
  );
}

/** The detail behind the meters: each limit's total, and each CLI proxy account's share of it. Refresh lives here. */
function UsageDetails({ usage, now, refreshing, refresh }: { usage: UsageLimits | null; now: number; refreshing: boolean; refresh: () => void }) {
  const ls = usage ? limits(usage) : [];
  const several = (usage?.accounts.length ?? 0) > 1;
  return (
    <div className={`ud ${refreshing ? "ud-refreshing" : ""}`}>
      <div className="ud-head">
        <span className="ud-title">{several ? `Claude usage · pool of ${usage?.accounts.length} accounts` : "Claude usage"}</span>
        <span className="ud-checked">{usage?.observedAt ? `Checked ${relTime(usage.observedAt, now)}` : "Not checked yet"}</span>
        <button className="btn btn-ghost btn-icon btn-sm" onClick={refresh} disabled={refreshing} aria-label="Refresh" data-tip="Refresh"><Icon name="refresh" size={15} /></button>
      </div>
      {ls.map((l) => {
        const resets = resetsLabel(l.total, new Date(now));
        const tiles = usage?.accounts.flatMap((a) => { const w = l.each(a); return w ? [{ a, w }] : []; }) ?? [];
        return (
          <section key={l.label} className="ud-limit">
            <div className={`ud-sum${hot(l.total)}`}>
              <span className="ud-label">{l.label}</span>
              <span className="ud-big num">{pct(l.total)}</span>
              <span className="ud-used">used</span>
              {resets ? <span className="ud-resets"><Icon name="clock" size={11} />{several ? resets.replace(/^Resets/, "First reset") : resets}</span> : null}
            </div>
            {tiles.length ? <div className="ud-tiles">{tiles.map(({ a, w }, i) => <Tile key={`${i}-${a.name}`} a={a} w={w} now={now} />)}</div> : null}
          </section>
        );
      })}
    </div>
  );
}

/** API-key accounts have no usage limits, so the card counts today's runs instead. */
function RunsToday() {
  const runs = useQuery(() => call("runs", { employeeId: null, limit: 500 }), [], ["run:updated"]);
  const startToday = new Date().setHours(0, 0, 0, 0);
  const n = (runs.data ?? []).filter((r) => Date.parse(r.createdAt) >= startToday).length;
  return (
    <>
      <div className="usage" aria-label="Runs today" title="API-key accounts have no usage limits">
        <div className="usage-head"><span>Runs today</span><span className="mono num">{n}</span></div>
      </div>
      <div className="usage-mini" data-tip={`${n} ${n === 1 ? "run" : "runs"} today`} aria-hidden>
        <span className="usage-mini-cost mono num">{n}</span>
      </div>
    </>
  );
}

/** Routines wait for the account the office was set up with; the button makes the signed-in one the office's own. */
function AccountIssue({ message, signedInAs }: { message: string; signedInAs: string | null }) {
  const [busy, setBusy] = useState(false);
  const use = () => {
    setBusy(true);
    call("use_signed_in_account").catch((e) => toast(errorMessage(e), "err")).finally(() => setBusy(false));
  };
  return (
    <div className="usage-note usage-note-warn">
      <span>{message}</span>
      {signedInAs ? <button className="btn btn-sm" onClick={use} disabled={busy}>Use {signedInAs} for the office</button> : null}
    </div>
  );
}

/** Sidebar card: Claude's 5-hour and 7-day usage limits. A click opens the detail beside it. */
export function UsageCard() {
  const { usage, refreshing, refresh } = useUsage();
  const now = useNow(60_000);
  const [open, setOpen] = useState(false);
  // The meters or the rail's mini meter, whichever was clicked: the detail opens beside it.
  const anchor = useRef<HTMLElement | null>(null);
  const toggle = (e: React.MouseEvent<HTMLElement>) => {
    anchor.current = e.currentTarget;
    setOpen((o) => !o);
  };

  if (usage && !usage.subscription) {
    // Its trigger goes with the meters, so the detail must not come back open beside nothing.
    if (open) setOpen(false);
    return <RunsToday />;
  }

  const five = usage?.fiveHour ?? null;
  const week = usage?.sevenDay ?? null;
  const known = !!usage?.observedAt;
  // The limit you will reach first, for the rail.
  const top = five && week ? (week.percent > five.percent ? week : five) : five ?? week;
  const checked = usage?.observedAt ? `Checked ${relTime(usage.observedAt, now)}` : "Not checked yet";
  const held = usage?.heldUntil && Date.parse(usage.heldUntil) > now ? new Date(usage.heldUntil) : null;
  // A hold from the 7-day limit can end days away, so another day is named.
  const heldDay = held && held.toDateString() !== new Date(now).toDateString() ? `${held.toLocaleDateString(undefined, { weekday: "short" })} ` : "";
  const summary = `Claude usage: 5-hour ${said(five, known)}, 7-day ${said(week, known)}`;
  return (
    <>
      <div className="usage" aria-label="Claude usage limits">
        <button className="usage-open" onClick={toggle} aria-haspopup="dialog" aria-expanded={open} aria-label={`${summary}. Show details`}>
          <Window label="5-hour" w={five} now={now} known={known} checked={checked} near={top === five} other={`7-day ${said(week, known)}`} />
          <Window label="7-day" w={week} now={now} known={known} checked={checked} near={top === week && top !== five} other={`5-hour ${said(five, known)}`} />
        </button>
        {held ? <div className="usage-note usage-note-warn">{usage?.heldForRateLimit
          ? <>Claude is turning requests away for now, so work starts again {heldDay}at {clockHM(held.getHours(), held.getMinutes())}.</>
          : <>Work starts again {heldDay}at {clockHM(held.getHours(), held.getMinutes())}, when the limit resets.</>}</div> : null}
        {usage?.accountIssue ? <AccountIssue message={usage.accountIssue.message} signedInAs={usage.accountIssue.signedInAs} /> : null}
        {usage?.proxyIssue ? <div className="usage-note usage-note-warn">{usage.proxyIssue}</div> : null}
      </div>
      {/* The rail: the nearer limit as a number over its meter, so it reads as a reading, not a menu glyph. */}
      <button className={`usage-mini ${level(top)}`} onClick={toggle} aria-haspopup="dialog" aria-expanded={open} data-tip={`Claude usage: 5-hour ${said(five, known)} · 7-day ${said(week, known)}`} aria-label={`${summary}. Show details`}>
        <span className="usage-mini-n num">{pct(top)}</span>
        <span className="usage-mini-bar"><span style={{ transform: `scaleX(${top ? Math.min(100, top.percent) / 100 : 0})` }} /></span>
      </button>
      <Popover anchor={anchor} open={open} onClose={() => setOpen(false)} side="right" width={500} label="Claude usage">
        <UsageDetails usage={usage} now={now} refreshing={refreshing} refresh={refresh} />
      </Popover>
    </>
  );
}
