// The shelves as the sidebar shows them right now, shared by the sidebar and the conversation screen: the data,
// your marks, which shelves you collapsed, the selected row and where it was when you selected it.
import { useEffect, useMemo, useState } from "react";
import type { Thread } from "../../api/types";
import { runKindLabel } from "../../lib/format";
import { refreshConversations, useConversations, useOrg, useProposals } from "../../lib/hooks";
import { chatKey, pruneMarks, shelvesSince, useMarks, type Marks } from "../../lib/marks";
import { openChat, useRoute, type Route } from "../../lib/router";
import { buildShelves, nextWake, visibleRows, type Placed, type Row, type ShelfId, type ShelvesView, type Sticky } from "../../lib/shelves";
import { storageGet, storageSet } from "../../lib/storage";
import { createStore, useStore } from "../../lib/store";
import { parseThreadFocus, threadFocus } from "../../lib/threads";
import { kindNames } from "../RunRow";

const COLLAPSED_KEY = "omega.shelvesCollapsed";
const COLLAPSED_DEFAULT: ShelfId[] = ["snoozed", "done"];

/** The shelves you collapsed, as saved. */
function loadCollapsed(): ReadonlySet<ShelfId> {
  try {
    const v: unknown = JSON.parse(storageGet(COLLAPSED_KEY) ?? "null");
    return new Set(Array.isArray(v) ? (v as ShelfId[]) : COLLAPSED_DEFAULT);
  } catch {
    return new Set(COLLAPSED_DEFAULT);
  }
}

const collapsedStore = createStore(loadCollapsed());
const openAllStore = createStore(false);
const stickyStore = createStore<Sticky>(null);

/** Opens or collapses a shelf, and remembers it. */
export function toggleShelf(id: ShelfId) {
  const next = new Set(collapsedStore.get());
  if (next.has(id)) next.delete(id);
  else next.add(id);
  collapsedStore.set(next);
  storageSet(COLLAPSED_KEY, JSON.stringify([...next]));
}

/** Shows every row of Open, or only the first few. */
export const showAllOpen = (all: boolean) => openAllStore.set(all);

/** After an explicit action (pin, snooze, done, reopen), the selected row goes where it now belongs at once. */
export const unstick = () => stickyStore.set(null);

/** The conversation key the route has open: a thread's root run id, `chat:<employee id>`, or null. */
export function routeKey(route: Route): string | null {
  if (route.page !== "chat") return null;
  return route.focus === null ? chatKey(route.employeeId) : parseThreadFocus(route.focus);
}

/** Opens a thread or ongoing chat row. A proposal row opens a sheet instead, so its caller handles it. */
export function openRow(row: Exclude<Row, { kind: "proposal" }>) {
  if (row.kind === "chat") openChat(row.employee.id);
  else openChat(row.employee.id, threadFocus(row.key));
}

/** Now, kept current at the next snooze's end and at midnight, when Done today starts over. */
function useWakeClock(marks: Marks): number {
  const [now, setNow] = useState(Date.now);
  useEffect(() => {
    const cur = Date.now();
    const midnight = new Date(cur);
    midnight.setHours(24, 0, 0, 0);
    const at = Math.min(nextWake(marks, cur) ?? Infinity, midnight.getTime());
    const h = window.setTimeout(() => {
      if (new Date().toDateString() !== new Date(cur).toDateString()) refreshConversations();
      setNow(Date.now());
    }, at - cur + 50);
    return () => window.clearTimeout(h);
  }, [marks, now]);
  return now;
}

/** The shelves, the rows on screen in order, and the selected row with where it sits. Null shelves while loading. */
export function useShelves() {
  const { data, error } = useConversations();
  const { snap } = useOrg();
  const proposals = useProposals();
  const marks = useMarks();
  const now = useWakeClock(marks);
  const collapsed = useStore(collapsedStore);
  const openAll = useStore(openAllStore);
  const sticky = useStore(stickyStore);
  const selected = routeKey(useRoute());
  const [since] = useState(shelvesSince);

  const shelves = useMemo(() => {
    if (!data || !snap) return null;
    const label = (t: Thread) => runKindLabel(t.root.kind, kindNames(t.root, snap.employees, snap.people));
    const open = (proposals ?? []).filter((p) => p.state === "open");
    const input = { conversations: data, employees: snap.employees, proposals: open, marks, since, label };
    return buildShelves(input, now, sticky && sticky.key === selected ? sticky : null);
  }, [data, snap, proposals, marks, since, now, sticky, selected]);
  const view = useMemo<ShelvesView>(() => ({ collapsed, openAll }), [collapsed, openAll]);
  const visible = useMemo(() => (shelves ? visibleRows(shelves, view) : []), [shelves, view]);
  const current: Placed | null = useMemo(
    () => (selected && shelves ? shelves.flatMap((s) => s.rows).find((p) => p.row.key === selected) ?? null : null),
    [shelves, selected],
  );
  return { shelves, error, view, visible, selected, current };
}

/** Keeps the selected row on the shelf it had when you selected it, and forgets marks of conversations that are gone.
 *  The sidebar runs this once. */
export function useShelfUpkeep(shelves: ReturnType<typeof useShelves>["shelves"], selected: string | null) {
  const sticky = useStore(stickyStore);
  useEffect(() => {
    if (!shelves || !selected || sticky?.key === selected) return;
    const shelf = shelves.find((s) => s.rows.some((p) => p.row.key === selected))?.id;
    if (shelf) stickyStore.set({ key: selected, shelf });
  }, [shelves, selected, sticky]);

  const { data } = useConversations();
  useEffect(() => {
    if (!data) return;
    const keep = new Set([...data.threads, ...data.done].map((t) => t.root.id));
    for (const c of data.chats) keep.add(chatKey(c.employeeId));
    pruneMarks(keep);
  }, [data]);
}
