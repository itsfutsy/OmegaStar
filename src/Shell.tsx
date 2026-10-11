// The app once you have a company: sidebar, the current page, and the sheets and palette any page can open.
import { createContext, useCallback, useContext, useEffect, useMemo, useState, type ReactNode } from "react";
import type { HireProposal } from "./api/types";
import { Icon, LogoMark, type IconName } from "./components/Icon";
import { CommandPalette } from "./components/CommandPalette";
import { HireSheet } from "./components/HireSheet";
import { AddPersonSheet } from "./components/AddPersonSheet";
import { UpdateCard } from "./components/UpdateCard";
import { UsageCard } from "./components/UsageCard";
import { Avatar } from "./components/ui";
import { usePruneMarks, useStatuses } from "./components/conversation/useStatuses";
import { firstName, initials } from "./lib/format";
import { openMember, openYou, useRoute } from "./lib/router";
import { YOU_ID } from "./lib/org";
import { useOrg, useProposals } from "./lib/hooks";
import { storageGet, storageSet } from "./lib/storage";
import { OrgScreen } from "./screens/Org";
import { ConversationScreen } from "./screens/Conversation";
import { InboxScreen } from "./screens/Inbox";
import { ActivityScreen } from "./screens/Activity";
import { SettingsScreen } from "./screens/Settings";
import { RunView } from "./screens/RunView";

type Sheet =
  | { kind: "hire"; managerId: string | null; template: string | null }
  | { kind: "proposal"; proposal: HireProposal }
  | { kind: "person"; managerId: string | null }
  | null;
type ShellApi = {
  hire: (managerId?: string | null, template?: string | null) => void;
  reviewProposal: (p: HireProposal) => void;
  addPerson: (managerId?: string | null) => void;
  palette: () => void;
};
const ShellCtx = createContext<ShellApi>({ hire: () => {}, reviewProposal: () => {}, addPerson: () => {}, palette: () => {} });
export const useShell = () => useContext(ShellCtx);

const MOD = typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.platform) ? "⌘" : "Ctrl";
const COLLAPSE_KEY = "omega.sidebarCollapsed";

export function Shell() {
  const route = useRoute();
  const { snap } = useOrg();
  const [sheet, setSheet] = useState<Sheet>(null);
  // Each opening remounts the sheet, so it starts from a fresh form.
  const [sheetKey, setSheetKey] = useState(0);
  const [palette, setPalette] = useState(false);
  const [collapsed, setCollapsed] = useState(() => storageGet(COLLAPSE_KEY) === "1");
  const { items } = useStatuses();
  usePruneMarks();

  const api = useMemo<ShellApi>(() => {
    const open = (s: Sheet) => { setSheet(s); setSheetKey((k) => k + 1); };
    return {
      hire: (managerId = null, template = null) => open({ kind: "hire", managerId, template }),
      reviewProposal: (proposal) => open({ kind: "proposal", proposal }),
      addPerson: (managerId = null) => open({ kind: "person", managerId }),
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
  const waiting = (snap?.people ?? []).reduce((s, p) => s + p.openRequests, 0) + (proposals ?? []).filter((p) => p.state === "open").length;
  const unread = (items ?? []).filter((i) => i.status.inbox && i.status.unread).length;
  const inboxCount = waiting + unread;

  const closeSheet = useCallback(() => setSheet(null), []);
  const toggleCollapsed = () => setCollapsed((c) => { storageSet(COLLAPSE_KEY, c ? "0" : "1"); return !c; });
  const company = snap?.company;

  let page: ReactNode;
  switch (route.page) {
    case "org": page = <OrgScreen route={route} />; break;
    case "chat": page = <ConversationScreen route={route} />; break;
    case "inbox": page = <InboxScreen />; break;
    case "activity": page = <ActivityScreen view={route.view} />; break;
    case "settings": page = <SettingsScreen />; break;
    case "run": page = <RunView key={route.runId} runId={route.runId} />; break;
  }
  const memberSelected = route.page === "org" ? route.memberId : null;
  const members = [
    ...(snap?.employees ?? []).map((e) => ({ id: e.id, name: e.name, hue: e.hue, person: false, dot: e.presence === "working" || e.presence === "queued" ? "live" : e.presence === "attention" ? "err" : null, working: e.working })),
    ...(snap?.people ?? []).map((p) => ({ id: p.id, name: p.name, hue: p.hue, person: true, dot: null, working: 0 })),
  ];

  return (
    <ShellCtx.Provider value={api}>
      <div className={`shell ${collapsed ? "shell-collapsed" : ""}`}>
        <aside className="side" aria-label="Sidebar">
          <div className="side-top">
            <a href="#/org" className="side-brand" aria-label="Omega Star">
              <LogoMark size={24} />
              <span className="side-word">Omega Star</span>
            </a>
            <button className="side-collapse" onClick={toggleCollapsed} aria-label={collapsed ? "Expand sidebar" : "Collapse sidebar"} data-tip={collapsed ? "Expand" : undefined}>
              <Icon name="sidebar" size={15} />
            </button>
          </div>
          <button className="side-search" onClick={() => setPalette(true)} data-tip={collapsed ? `Search (${MOD} K)` : undefined} aria-label="Search and commands">
            <Icon name="search" size={15} />
            <span className="side-text">Search</span>
            <span className="kbd side-kbd">{MOD} K</span>
          </button>
          <nav className="side-nav">
            <NavItem href="#/org" icon="org" label="Org chart" active={route.page === "org"} collapsed={collapsed} />
            <NavItem href="#/inbox" icon="inbox" label="Inbox" active={route.page === "inbox"} badge={inboxCount || null} collapsed={collapsed} />
            <NavItem href="#/activity" icon="activity" label="Activity" active={route.page === "activity" || route.page === "run"} collapsed={collapsed} />
            {members.length ? <div className="side-label">Team</div> : null}
            <div className="side-team">
              {members.slice(0, 12).map((m) => (
                <button key={m.id} className={`side-item side-member ${memberSelected === m.id ? "side-item-on" : ""}`} onClick={() => openMember(m.id)} data-tip={collapsed ? m.name : undefined} aria-label={m.name}>
                  <span className="side-member-av">
                    <Avatar name={m.name} hue={m.hue} size={20} person={m.person} />
                    {m.dot ? <span className={`side-dot side-dot-${m.dot}`} /> : null}
                  </span>
                  <span className="side-text">{m.person ? m.name : firstName(m.name)}</span>
                  {m.working > 1 ? <span className="side-count num" title={`Working on ${m.working} things`}>{m.working}</span> : null}
                </button>
              ))}
              {members.length > 12 ? <a href="#/org" className="side-item side-more"><span className="side-text">{members.length - 12} more in the org chart</span></a> : null}
            </div>
          </nav>
          <div className="side-bottom">
            <UpdateCard />
            <NavItem href="#/settings" icon="settings" label="Settings" active={route.page === "settings"} collapsed={collapsed} />
            <UsageCard />
            {company ? (
              <button className={`side-you ${memberSelected === YOU_ID ? "side-you-on" : ""}`} onClick={openYou} data-tip={collapsed ? company.ceoName : undefined} aria-label={`${company.ceoName}, ${company.title} of ${company.name}. Edit your profile`}>
                <span className="side-you-av">{initials(company.ceoName)}</span>
                <span className="side-you-text">
                  <span className="side-you-name">{company.ceoName}</span>
                  <span className="side-you-sub">{company.title} · {company.name}</span>
                </span>
              </button>
            ) : null}
          </div>
        </aside>
        <main className="content">
          <div className="page">{page}</div>
        </main>
      </div>
      <HireSheet
        key={`hire-${sheetKey}`}
        open={sheet?.kind === "hire" || sheet?.kind === "proposal"}
        managerId={sheet?.kind === "hire" ? sheet.managerId : null}
        template={sheet?.kind === "hire" ? sheet.template : null}
        proposal={sheet?.kind === "proposal" ? sheet.proposal : null}
        onClose={closeSheet}
      />
      <AddPersonSheet key={`person-${sheetKey}`} open={sheet?.kind === "person"} managerId={sheet?.kind === "person" ? sheet.managerId : null} onClose={closeSheet} />
      {palette ? <CommandPalette onClose={() => setPalette(false)} api={api} /> : null}
    </ShellCtx.Provider>
  );
}

/** One of the sidebar's pages, with an optional count; its name moves to a tooltip when the sidebar is collapsed. */
function NavItem({ href, icon, label, active, badge, collapsed }: { href: string; icon: IconName; label: string; active: boolean; badge?: number | null; collapsed: boolean }) {
  return (
    <a href={href} className={`side-item ${active ? "side-item-on" : ""}`} aria-current={active ? "page" : undefined} data-tip={collapsed ? (badge ? `${label} · ${badge}` : label) : undefined}>
      <Icon name={icon} size={16} />
      <span className="side-text">{label}</span>
      {badge ? <span className="side-badge num" aria-label={`${badge} waiting`}>{badge}</span> : null}
    </a>
  );
}
