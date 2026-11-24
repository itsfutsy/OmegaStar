import { useSyncExternalStore } from "react";
import type { Run } from "../api/types";
import { YOU_ID } from "./org";
import { conversationOf, threadFocus } from "./threads";

/** An employee panel's tabs. People and you have a single page, so no tab. You talk to an employee in the
 *  conversation, not on their page. */
export type PanelTab = "work" | "skills" | "profile";
export type ActivityView = "runs" | "numbers" | "skills";
/** The Inbox's tabs. */
export type InboxView = "inbox" | "snoozed" | "archive";
const INBOX_VIEWS: InboxView[] = ["inbox", "snoozed", "archive"];

export type Route =
  /** The org chart, with a member's panel open. On the Skills tab, `focus` says which file or change is open. */
  | { page: "org"; memberId: string | null; tab: PanelTab | null; focus: string | null }
  /** A conversation with an employee, the one place you talk to them. `focus`: `thread~<root run id>`, or null for a new
   *  conversation. */
  | { page: "chat"; employeeId: string; focus: string | null }
  /** The Inbox, on a tab, with one item open: `item` is the id the Inbox gives its entries (a Decision's id, or a hire
   *  proposal's). Both optional: `#/inbox` is the Inbox tab with the first item. */
  | { page: "inbox"; view?: InboxView; item?: string | null }
  | { page: "activity"; view: ActivityView }
  | { page: "settings" }
  /** The Analytics page: how big the team is and how much it gets done, in the sidebar under the Org chart. */
  | { page: "analytics" }
  /** A run (from `openRun`, or an old link to a run's own page): it opens the run's conversation. */
  | { page: "run"; runId: string };

const TABS: PanelTab[] = ["work", "skills", "profile"];

function parseHash(hash: string): Route {
  const parts = hash.replace(/^#\/?/, "").split("/").filter(Boolean).map(decodeURIComponent);
  switch (parts[0]) {
    case "inbox": return { page: "inbox", view: INBOX_VIEWS.find((v) => v === parts[1]) ?? "inbox", item: parts[2] ?? null };
    case "activity": return { page: "activity", view: parts[1] === "skills" || parts[1] === "numbers" ? parts[1] : "runs" };
    case "settings": return { page: "settings" };
    case "analytics": return { page: "analytics" };
    case "run": if (parts[1]) return { page: "run", runId: parts[1] }; break;
    case "chat": if (parts[1]) return { page: "chat", employeeId: parts[1], focus: parts[2] ?? null }; break;
  }
  // An old link to an employee's Chat tab is their conversation.
  if (parts[1] && parts[2] === "chat") return { page: "chat", employeeId: parts[1], focus: parts[3] ?? null };
  const tab = TABS.find((t) => t === parts[2]) ?? null;
  return { page: "org", memberId: parts[1] ?? null, tab, focus: parts[3] ?? null };
}

function routeHash(r: Route): string {
  switch (r.page) {
    case "org": {
      const segs = ["org"];
      if (r.memberId) {
        segs.push(r.memberId);
        if (r.tab) segs.push(r.tab);
        if (r.tab && r.focus) segs.push(r.focus);
      }
      return "#/" + segs.map(encodeURIComponent).join("/");
    }
    case "run": return `#/run/${encodeURIComponent(r.runId)}`;
    case "chat": return "#/" + ["chat", r.employeeId, ...(r.focus ? [r.focus] : [])].map(encodeURIComponent).join("/");
    case "activity": return r.view === "runs" ? "#/activity" : `#/activity/${r.view}`;
    case "inbox": {
      const view = r.view ?? "inbox";
      if (view === "inbox" && !r.item) return "#/inbox";
      return "#/" + ["inbox", view, ...(r.item ? [r.item] : [])].map(encodeURIComponent).join("/");
    }
    default: return `#/${r.page}`;
  }
}

export function navigate(r: Route, opts: { replace?: boolean } = {}) {
  const h = routeHash(r);
  if (h === location.hash) return;
  if (opts.replace) {
    history.replaceState(null, "", h);
    window.dispatchEvent(new HashChangeEvent("hashchange"));
  } else {
    location.hash = h;
  }
}

export const openMember = (memberId: string, tab: PanelTab | null = null, focus: string | null = null, replace = false) =>
  navigate({ page: "org", memberId, tab, focus }, { replace });
/** A conversation with an employee: with `focus` a thread, without one a new conversation. */
export const openChat = (employeeId: string, focus: string | null = null, replace = false) =>
  navigate({ page: "chat", employeeId, focus }, { replace });
/** A run where its work is: the conversation it is part of (the run route finds it). */
export const openRun = (runId: string) => navigate({ page: "run", runId });
/** The Inbox on `view`, with `item` open (its entry's id). `replace` for moving between items without history. */
export const openInbox = (view: InboxView = "inbox", item: string | null = null, replace = false) =>
  navigate({ page: "inbox", view, item }, { replace });
/** A run where you can follow it: its conversation, the thread it is a turn of or the one it starts. */
export function openWork(run: Run) {
  openChat(run.employeeId, threadFocus(conversationOf(run) ?? run.id));
}
/** Your own panel on the org chart: name, title, company and who you report to. */
export const openYou = () => openMember(YOU_ID);

function subscribe(cb: () => void) {
  window.addEventListener("hashchange", cb);
  return () => window.removeEventListener("hashchange", cb);
}

// One Route object per hash, so a re-render without navigation hands back the same route.
let lastHash = "\u0000";
let lastRoute: Route = { page: "org", memberId: null, tab: null, focus: null };
function routeOf(hash: string): Route {
  if (hash !== lastHash) {
    lastHash = hash;
    lastRoute = parseHash(hash);
  }
  return lastRoute;
}

export function useRoute(): Route {
  return routeOf(useSyncExternalStore(subscribe, () => location.hash));
}
