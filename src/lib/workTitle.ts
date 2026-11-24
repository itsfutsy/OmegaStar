import type { EmployeeView, PersonView, Run } from "../api/types";
import { capitalize, firstName, runKindLabel } from "./format";

export function kindNames(run: Run, employees: EmployeeView[], people: PersonView[]): { manager?: string; person?: string; asker?: string } {
  if (run.kind.type === "asked" || run.kind.type === "answered") {
    const other = run.kind.type === "asked" ? run.kind.askedBy : run.kind.from;
    const a = employees.find((x) => x.id === other);
    return { asker: a ? firstName(a.name) : undefined };
  }
  if (run.kind.type === "delegated") {
    const e = employees.find((x) => x.id === run.employeeId);
    const m = e?.managerId ? employees.find((x) => x.id === e.managerId) : undefined;
    return { manager: m ? firstName(m.name) : undefined };
  }
  if (run.kind.type === "followUp" || run.kind.type === "replyCheck" || run.kind.type === "nudge" || run.kind.type === "unanswered") {
    const personId = run.kind.personId;
    const p = people.find((x) => x.id === personId);
    return { person: p ? firstName(p.name) : undefined };
  }
  return {};
}

/** The prompt of a run started with no instruction. Mirrors Rust `DEFAULT_PROMPT`. */
const DEFAULT_PROMPT = "Work toward your objective. Start from your role skill, pick the most valuable next step, do it, and report.";

/** A prompt's first line. */
const firstPromptLine = (prompt: string) => prompt.split("\n").find((l) => l.trim())?.trim() ?? "";

/** A title from an instruction: its first sentence, without the full stop a title doesn't carry ("Check the booking
 *  widget on staging", not "Check the booking widget on staging. Screenshots of anything off."). */
function titleOf(line: string): string {
  const first = line.match(/^.+?[.!?](?=\s+[A-Z(])/)?.[0] ?? line;
  return first.replace(/\.$/, "").trim();
}

/** What a run is about, in a few words: a manual run you gave an instruction is its first sentence, and work a manager
 *  handed down is its task, never who handed it over (that is the row's second line). */
export function workLabel(run: Run, employees: EmployeeView[], people: PersonView[]): string {
  if ((run.kind.type === "manual" && run.prompt !== DEFAULT_PROMPT) || run.kind.type === "delegated") {
    const line = firstPromptLine(run.prompt);
    if (line) return titleOf(line);
  }
  return runKindLabel(run.kind, kindNames(run, employees, people));
}

/** The first sentence of what was asked, whole: without "Please" and its closing full stop, never cut. */
function firstSentence(text: string): string {
  const line = text.split("\n").map((l) => l.trim().replace(/^[#>\-* ]+/, "").replace(/[*_`]/g, "")).find((l) => l) ?? "";
  const s = line.replace(/^(please|can you|could you|would you)[, ]+/i, "");
  const ends = [". ", "? ", "! "].map((e) => s.indexOf(e)).filter((i) => i >= 0);
  const out = (ends.length ? s.slice(0, Math.min(...ends)) : s).replace(/[?.!,:;]+$/, "").trim();
  return capitalize(out);
}

/** What a piece of work is called where a person's state is shown: the conversation's name without "Re:", the task or
 *  question handed over without "Delegated by" or "Task from", in its own first sentence. */
export function workTitle(run: Run, employees: EmployeeView[], people: PersonView[]): string {
  if (run.title) return run.title;
  switch (run.kind.type) {
    case "reply": case "task": return run.kind.label;
    case "delegated": case "asked": return firstSentence(run.prompt) || workLabel(run, employees, people);
    case "manual": {
      const w = workLabel(run, employees, people);
      return w === "Manual run" ? w : firstSentence(run.prompt) || w;
    }
    default: return workLabel(run, employees, people);
  }
}

/** A task short enough for a card: whole, or cut at its last clause break before `max` ("Check which price IDs change on
 *  1 March"), else at a word with an ellipsis. */
export function shortTask(text: string, max = 52): string {
  if (text.length <= max) return text;
  const head = text.slice(0, max);
  const breaks = [", ", "; ", " and ", " then ", " so ", " but "].map((b) => head.lastIndexOf(b)).filter((i) => i >= 16);
  if (breaks.length) return text.slice(0, Math.max(...breaks));
  return `${head.slice(0, head.lastIndexOf(" "))}…`;
}
