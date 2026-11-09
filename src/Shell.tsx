// The app once you have a company: sidebar, the current page, and the sheets and palette any page can open.
import { createContext, useCallback, useContext, useEffect, useLayoutEffect, useMemo, useState, type ReactNode } from "react";
import type { HireProposal } from "./api/types";
import { Icon, LogoMark, type IconName } from "./components/Icon";
import { CommandPalette } from "./components/CommandPalette";
import { HireSheet } from "./components/HireSheet";
import { AddPersonSheet } from "./components/AddPersonSheet";
import { UpdateCard } from "./components/UpdateCard";
import { UsageCard } from "./components/UsageCard";
import { workLabel } from "./components/RunRow";
import { Avatar, Roll } from "./components/ui";
import { conversationFor, usePruneMarks, useStatuses } from "./components/conversation/useStatuses";
import { decisions } from "./lib/decisions";
import { firstName, initials, MOD } from "./lib/format";
import { openChat, openYou, useRoute } from "./lib/router";
import { YOU_ID } from "./lib/org";
import { useOrg, useProposals } from "./lib/hooks";
import { dragRegion, macChrome } from "./lib/chrome";
import { usePresence } from "./lib/motion";
import { usePending } from "./lib/pending";
import { useScrollFade } from "./lib/scrollFade";
import { useRail } from "./lib/sidebar";
import { OrgScreen } from "./screens/Org";
import { ConversationScreen, RunRedirect } from "./screens/Conversation";
import { InboxScreen } from "./screens/Inbox";
import { ActivityScreen } from "./screens/Activity";
import { SettingsScreen } from "./screens/Settings";

type Sheet =
  | { kind: "hire"; managerId: string | null; template: string | null }
  | { kind: "proposal"; proposal: HireProposal }
  | { kind: "person"; managerId: string | null }
  | null;
/** `from`: the button that opened a sheet, so focus goes back to it on close (WebKit doesn't focus a button on click). */
type ShellApi = {
  hire: (managerId?: string | null, template?: string | null, from?: HTMLElement | null) => void;
  reviewProposal: (p: HireProposal, from?: HTMLElement | null) => void;
  addPerson: (managerId?: string | null, from?: HTMLElement | null) => void;
  palette: () => void;
};
const ShellCtx = createContext<ShellApi>({ hire: () => {}, reviewProposal: () => {}, addPerson: () => {}, palette: () => {} });
export const useShell = () => useContext(ShellCtx);

/** The palette's leaving fade (--d-ui), with a frame to spare. */
const PALETTE_LEAVE_MS = 160;

/** How long the sidebar's arrival takes at most: its last row's delay and its own drop. */
const ARRIVE_MS = 1200;

/** `arriving`: onboarding has just handed over, so the canvas comes in as one body swap and the sidebar's parts drop
 *  into the band in reading order (your row, already there in onboarding's band, stays put). */
export function Shell({ arriving = false }: { arriving?: boolean }) {
  // Only for the hand-off itself: a row that arrives later (someone starts working) is not part of it.
  const [arrive, setArrive] = useState(arriving);
  useEffect(() => {
    if (!arrive) return;
    const t = window.setTimeout(() => setArrive(false), ARRIVE_MS);
    return () => window.clearTimeout(t);
  }, [arrive]);
  const route = useRoute();
  const { snap } = useOrg();
  const [sheet, setSheet] = useState<Sheet>(null);
  // Each opening remounts the sheet, so it starts from a fresh form.
  const [sheetKey, setSheetKey] = useState(0);
  // The button that opened the last sheet: kept past the close, which is when focus goes back to it.
  const [sheetFrom, setSheetFrom] = useState<HTMLElement | null>(null);
  const [palette, setPalette] = useState(false);
  // Below 1180 px the sidebar is the rail unless you expanded it there; above, your saved choice (src/lib/sidebar.ts).
  const { rail: collapsed, toggle: toggleCollapsed } = useRail();
  const { items } = useStatuses();
  usePruneMarks();
  // The palette fades where it is when it closes; each opening starts it afresh.
  const paletteShown = usePresence(palette, PALETTE_LEAVE_MS);
  const sideRef = useScrollFade<HTMLElement>();
  const navRef = useScrollFade<HTMLElement>();
  const tight = useTight(navRef);
  const [paletteOpens, setPaletteOpens] = useState({ open: palette, n: 0 });
  if (paletteOpens.open !== palette) setPaletteOpens({ open: palette, n: paletteOpens.n + (palette ? 1 : 0) });

  const api = useMemo<ShellApi>(() => {
    const open = (s: Sheet, from: HTMLElement | null) => { setSheet(s); setSheetFrom(from); setSheetKey((k) => k + 1); };
    return {
      hire: (managerId = null, template = null, from = null) => open({ kind: "hire", managerId, template }, from),
      reviewProposal: (proposal, from = null) => open({ kind: "proposal", proposal }, from),
      addPerson: (managerId = null, from = null) => open({ kind: "person", managerId }, from),
      palette: () => setPalette(true),
    };
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "k") { e.preventDefault(); setPalette((p) => !p); }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
  const proposals = useProposals();
  // The badge counts decisions: hire proposals, and every entry the Inbox shows, less the answers still on their way
  // (inside an Undo), so it always matches the Inbox's own count. The dock shows it too.
  const pending = usePending();
  const inboxCount = (proposals ?? []).filter((p) => p.state === "open" && !pending.has(p.id)).length
    + decisions((items ?? []).filter((i) => i.status.inbox)).filter((d) => !pending.has(d.id)).length;
  useEffect(() => { void dockBadge(inboxCount); }, [inboxCount]);

  // Toasts and other window-level layers centre on the canvas, not the window, so they say which sidebar is showing.
  useEffect(() => {
    const root = document.documentElement;
    root.dataset.shell = collapsed ? "rail" : "side";
    return () => { delete root.dataset.shell; };
  }, [collapsed]);

  const closeSheet = useCallback(() => setSheet(null), []);
  const company = snap?.company;

  let page: ReactNode;
  switch (route.page) {
    case "org": page = <OrgScreen route={route} />; break;
    case "chat": page = <ConversationScreen route={route} />; break;
    case "inbox": page = <InboxScreen />; break;
    case "activity": page = <ActivityScreen view={route.view} />; break;
    case "settings": page = <SettingsScreen />; break;
    case "run": page = <RunRedirect key={route.runId} runId={route.runId} />; break;
  }
  const memberSelected = route.page === "org" ? route.memberId : null;
  // Your team: every employee, in the chart's order, each a click from their conversation (the one that matters now:
  // what needs you, what they are working on, else their newest). Where each stands is one dot on their face, said in
  // words in the row's name and tooltip; the Inbox (Needs you) gathers what needs you. People are in the org chart.
  const open = route.page === "chat" ? route.employeeId : null;
  const employees = snap?.employees ?? [];
  const people = snap?.people ?? [];
  const team = employees.map((e) => {
    const what = e.live.map((r) => workLabel(r, employees, people));
    const on = (state: string) => what.filter((_, i) => e.live[i].state === state).join(", ");
    const [running, queued] = [on("running"), on("queued")];
    const doing = [running && `working on ${running}`, queued && `up next: ${queued}`].filter(Boolean).join("; ");
    const failed = e.presence === "attention" && (e.lastRun?.state === "failed" || e.lastRun?.state === "lost");
    const state = e.presence === "working" ? "working" : e.presence === "queued" ? "up next" : e.presence === "attention" ? (failed ? "failed" : "needs you") : null;
    const tone = e.presence === "working" ? "live" : e.presence === "queued" ? "wait" : e.presence === "attention" ? "attn" : null;
    const first = firstName(e.name);
    const said = doing || state;
    return {
      e, first, tone,
      label: said ? `${e.name}, ${said}` : e.name,
      tip: said ? `${collapsed ? `${first}: ` : ""}${said.charAt(0).toUpperCase() + said.slice(1)}` : collapsed ? first : undefined,
    };
  });
  return (
    <ShellCtx.Provider value={api}>
      <div className={`shell ${collapsed ? "shell-collapsed" : ""}`}>
        {macChrome ? <div className="side-chrome" {...dragRegion} /> : null}
        <aside ref={sideRef} className={`side scroll-fade ${arrive ? "side-arriving" : ""} ${tight ? "side-tight" : ""}`} aria-label="Sidebar">
          <div className="side-top">
            <SideBrand />
            <button className="side-collapse" onClick={toggleCollapsed} aria-label={collapsed ? "Expand sidebar" : "Collapse sidebar"} data-tip={collapsed ? "Expand" : undefined}>
              <Icon name="sidebar" size={15} />
            </button>
          </div>
          <button className="side-search" onClick={() => setPalette(true)} data-tip={collapsed ? `Search (${MOD} K)` : undefined} aria-label="Search and commands">
            <Icon name="search" size={15} />
            <span className="side-text">Search</span>
            <span className="kbd side-kbd">{MOD} K</span>
          </button>
          <nav ref={navRef} className="side-nav scroll-fade">
            <NavItem href="#/inbox" icon="inbox" label="Needs you" active={route.page === "inbox"} badge={inboxCount || null} collapsed={collapsed} />
            <NavItem href="#/org" icon="org" label="Org chart" active={route.page === "org"} collapsed={collapsed} />
            {team.length ? <div className="side-label">Team</div> : null}
            <div className="side-team">
              {team.map((m) => (
                <button key={m.e.id} className={`side-item side-member ${open === m.e.id ? "side-item-on" : ""}`} onClick={() => openChat(m.e.id, conversationFor(m.e, items))} data-tip={m.tip} aria-label={m.label} aria-current={open === m.e.id ? "page" : undefined}>
                  <span className="side-member-av">
                    <Avatar name={m.e.name} hue={m.e.hue} image={m.e.image} size={20} />
                    {m.tone ? <span className={`side-dot side-dot-${m.tone}`} /> : null}
                  </span>
                  <span className="side-text">{m.first}</span>
                </button>
              ))}
            </div>
          </nav>
          <div className="side-bottom">
            <UpdateCard />
            <NavItem href="#/settings" icon="settings" label="Settings" active={route.page === "settings"} collapsed={collapsed} />
            <UsageCard />
            {company ? (
              <button className={`side-you ${memberSelected === YOU_ID ? "side-you-on" : ""}`} onClick={openYou} data-tip={collapsed ? company.ceoName : undefined} aria-label={`${company.ceoName}, ${company.title} of ${company.name}. Edit your profile`}>
                <span className="side-you-av">{company.image ? <img className="av-img" src={company.image} alt="" draggable={false} /> : <span className="av-i">{initials(company.ceoName)}</span>}</span>
                <span className="side-you-text">
                  <span className="side-you-name">{company.ceoName}</span>
                  <span className="side-you-sub">{company.title} · {company.name}</span>
                </span>
              </button>
            ) : null}
          </div>
        </aside>
        <main className={`content ${arriving ? "enter-rise" : ""}`}>
          <div className="page">{page}</div>
        </main>
      </div>
      <HireSheet
        key={`hire-${sheetKey}`}
        open={sheet?.kind === "hire" || sheet?.kind === "proposal"}
        managerId={sheet?.kind === "hire" ? sheet.managerId : null}
        template={sheet?.kind === "hire" ? sheet.template : null}
        proposal={sheet?.kind === "proposal" ? sheet.proposal : null}
        returnFocus={sheetFrom}
        onClose={closeSheet}
      />
      <AddPersonSheet key={`person-${sheetKey}`} open={sheet?.kind === "person"} managerId={sheet?.kind === "person" ? sheet.managerId : null} onClose={closeSheet} />
      {paletteShown ? <CommandPalette key={paletteOpens.n} leaving={!palette} onClose={() => setPalette(false)} api={api} /> : null}
    </ShellCtx.Provider>
  );
}

/** Whether the usage card should fold to its nearer limit, so the state rows above fit whole: true while the list would
 *  overflow without the fold. Measured against the room the fold gives (one usage row), so it never flips back and
 *  forth. */
function useTight(nav: React.RefObject<HTMLElement | null>): boolean {
  const [tight, setTight] = useState(false);
  useLayoutEffect(() => {
    const el = nav.current;
    if (!el) return;
    const check = () => {
      const row = document.querySelector<HTMLElement>(".usage-row:not(.usage-near)");
      const gives = tight ? 24 : (row?.offsetHeight ?? 0) + 2;
      const over = el.scrollHeight - el.clientHeight;
      setTight(tight ? over + gives > 0 : over > 0 && gives > 0);
    };
    check();
    const ro = new ResizeObserver(check);
    ro.observe(el);
    for (const c of el.children) ro.observe(c);
    const mo = new MutationObserver(check);
    mo.observe(el, { childList: true, subtree: true });
    return () => { ro.disconnect(); mo.disconnect(); };
  }, [nav, tight]);
  return tight;
}

/** The count on the app's icon in the Dock (macOS), cleared at 0. Elsewhere it may not be supported, which is fine. */
async function dockBadge(count: number) {
  if (!("__TAURI_INTERNALS__" in window)) return;
  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  await getCurrentWindow().setBadgeCount(count > 0 ? count : undefined).catch(() => {});
}

/** The product's mark and name at the top of the sidebar; the boot screen draws it in the same place. */
export function SideBrand() {
  return (
    <a href="#/org" className="side-brand" aria-label="Omega Star">
      <LogoMark size={20} tile={false} />
      <span className="side-word">Omega Star</span>
    </a>
  );
}

/** One of the sidebar's pages, with an optional count that rolls when it changes; its name moves to a tooltip when the
 *  sidebar is collapsed. */
function NavItem({ href, icon, label, active, badge, collapsed }: { href: string; icon: IconName; label: string; active: boolean; badge?: number | null; collapsed: boolean }) {
  return (
    <a href={href} className={`side-item ${active ? "side-item-on" : ""}`} aria-current={active ? "page" : undefined} data-tip={collapsed ? (badge ? `${label} · ${badge}` : label) : undefined}>
      <Icon name={icon} size={16} />
      <span className="side-text">{label}</span>
      {badge ? <span className="side-badge num" aria-label={`${badge} waiting`}><Roll value={badge} /></span> : null}
    </a>
  );
}
