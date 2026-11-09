// The reporting hierarchy, mirroring src-tauri/src/org.rs: you are a node in it, every employee
// reports up to you, and only people sit above you. Every "Reports to" picker offers only the
// choices that pass `validateOrg`.
import type { Company, Employee, Person, PersonManager } from "../api/types";

/** Your node in the org chart, and the route id of your panel. Member ids are never "you". */
export const YOU_ID = "you";

type EmployeeLike = Pick<Employee, "id" | "name" | "managerId">;
type PersonLike = Pick<Person, "id" | "name" | "manager">;
type OrgShape<E extends EmployeeLike = EmployeeLike, P extends PersonLike = PersonLike> = {
  company: Pick<Company, "reportsTo">; employees: E[]; people: P[];
};

/** A person's manager as a node id: YOU_ID, a member id, or null for the top of the company. */
function personParent(m: PersonManager): string | null {
  switch (m.type) {
    case "you": return YOU_ID;
    case "member": return m.id;
    case "top": return null;
  }
}

/** The node directly above `id` (YOU_ID for you); null at the top of the company. */
export function up(o: OrgShape, id: string): string | null {
  if (id === YOU_ID) return o.company.reportsTo;
  const e = o.employees.find((x) => x.id === id);
  if (e) return e.managerId ?? YOU_ID;
  const p = o.people.find((x) => x.id === id);
  return p ? personParent(p.manager) : null;
}

/** Everyone above `from`, nearest first; null on a cycle. */
function chain(o: OrgShape, from: string): string[] | null {
  const seen = new Set([from]);
  const out: string[] = [];
  for (let cur = up(o, from); cur !== null; cur = up(o, cur)) {
    if (seen.has(cur)) return null;
    seen.add(cur);
    out.push(cur);
  }
  return out;
}

const reportsUpToYou = (o: OrgShape, id: string) => chain(o, id)?.includes(YOU_ID) ?? false;

/** The people you report up through: your manager first, the top of the company last. */
function lineAboveYou(o: OrgShape): string[] {
  return chain(o, YOU_ID) ?? [];
}

/** People who don't report up to you: the line above you and anyone under them. */
export function aboveYou(o: OrgShape): Set<string> {
  return new Set(o.people.filter((p) => !reportsUpToYou(o, p.id)).map((p) => p.id));
}

/** The hierarchy rules: null when the org is valid, otherwise why not. */
function validateOrg(o: OrgShape): string | null {
  const name = (id: string) => o.employees.find((e) => e.id === id)?.name ?? o.people.find((p) => p.id === id)?.name ?? "someone";
  const exists = (id: string) => o.employees.some((e) => e.id === id) || o.people.some((p) => p.id === id);
  const boss = o.company.reportsTo;
  if (boss !== null) {
    const agent = o.employees.find((e) => e.id === boss);
    if (agent) return `${agent.name} is an agent. Only a real person can sit above you: every agent reports up to you.`;
    if (!o.people.some((p) => p.id === boss)) return "The person you report to is not in the org chart.";
  }
  const managers = [
    ...o.employees.map((e) => [e.id, e.managerId] as const),
    ...o.people.map((p) => [p.id, p.manager.type === "member" ? p.manager.id : null] as const),
  ];
  for (const [id, m] of managers) {
    if (m === id) return `${name(id)} can't report to themselves.`;
    if (m !== null && !exists(m)) return "That manager is not in the org chart.";
  }
  for (const id of [YOU_ID, ...managers.map(([id]) => id)]) {
    if (chain(o, id) === null) return "That manager would create a reporting cycle.";
  }
  const stray = o.employees.find((e) => !reportsUpToYou(o, e.id));
  if (stray) return `${stray.name} is an agent, and every agent reports up to you. Only people can sit above you.`;
  const tops = o.people.filter((p) => p.manager.type === "top");
  if (boss === null && tops.length > 0) return `${tops[0].name} can't be at the top of the company: you are. Choose who they report to.`;
  if (boss !== null && tops.length > 1) return `${tops[1].name} can't also be at the top of the company. Only the person at the top of your reporting line has no manager.`;
  return null;
}

/**
 * Puts `to` directly above you (null: you become the top of the company), editing `o` in place.
 * Mirrors `Org::set_reports_to`: a person already above you keeps their manager; one from below
 * moves alone into your old place in the line. Their people stay with them; any agent left under
 * someone above you now reports to you. Clearing puts everyone on your line back under you.
 * Returns why it was refused, or null.
 */
function setReportsTo(o: OrgShape, to: string | null): string | null {
  if (o.company.reportsTo === to) return null;
  if (to === null) {
    for (const id of lineAboveYou(o)) {
      const p = o.people.find((x) => x.id === id);
      if (p) p.manager = { type: "you" };
    }
    o.company.reportsTo = null;
    return null;
  }
  const agent = o.employees.find((e) => e.id === to);
  if (agent) return `${agent.name} is an agent. Only a real person can sit above you: you stay in charge of every agent.`;
  const person = o.people.find((p) => p.id === to);
  if (!person) return "That person is not in the org chart.";
  if (!aboveYou(o).has(to)) person.manager = o.company.reportsTo === null ? { type: "top" } : { type: "member", id: o.company.reportsTo };
  o.company.reportsTo = to;
  const above = aboveYou(o);
  for (const e of o.employees) if (e.managerId !== null && above.has(e.managerId)) e.managerId = null;
  return null;
}

function copy(o: OrgShape): OrgShape {
  return {
    company: { reportsTo: o.company.reportsTo },
    employees: o.employees.map((e) => ({ id: e.id, name: e.name, managerId: e.managerId })),
    people: o.people.map((p) => ({ id: p.id, name: p.name, manager: p.manager })),
  };
}

const NEW_ID = "__new__";

/** Managers an employee may have: null (you) or a member id. `selfId` null = a new hire. */
export function employeeManagerOptions(o: OrgShape, selfId: string | null): (string | null)[] {
  const candidates = [null, ...o.employees.map((e) => e.id), ...o.people.map((p) => p.id)];
  return candidates.filter((m) => {
    const t = copy(o);
    const self = selfId ?? NEW_ID;
    t.employees = [...t.employees.filter((e) => e.id !== self), { id: self, name: "They", managerId: m }];
    return validateOrg(t) === null;
  });
}

/** Managers a person may have. `selfId` null = a person being added. */
export function personManagerOptions(o: OrgShape, selfId: string | null): PersonManager[] {
  const candidates: PersonManager[] = [
    { type: "you" }, { type: "top" },
    ...[...o.employees, ...o.people].map((m): PersonManager => ({ type: "member", id: m.id })),
  ];
  return candidates.filter((m) => {
    const t = copy(o);
    const self = selfId ?? NEW_ID;
    t.people = [...t.people.filter((p) => p.id !== self), { id: self, name: "They", manager: m }];
    return validateOrg(t) === null;
  });
}

/** What choosing `to` as your manager would change: the agents it moves to you. */
export function reportsToEffect(o: OrgShape, to: string | null): { error: string | null; agentsToYou: string[] } {
  const t = copy(o);
  const error = setReportsTo(t, to) ?? validateOrg(t);
  const moved = t.employees.filter((e) => e.managerId === null && o.employees.find((x) => x.id === e.id)?.managerId !== null);
  return { error, agentsToYou: moved.map((e) => e.name) };
}
