// What you sent from a conversation's composer before Omega Star has taken it: shown at once as your message on its way,
// then dropped the moment the conversation itself shows it, or kept with Retry when it did not go. ChatSurface holds
// the list; the rules for when one shows are here, as plain functions.
import type { NewAttachment, RunEvent } from "../api/types";
import type { LiveRun } from "./runs";

export type Outgoing = {
  /** Local only: never sent, never a run id. */
  id: number;
  text: string;
  /** The files' names, for the chips above the message. */
  files: string[];
  /** Read once at Send, so Retry sends exactly the same thing. */
  attachments: NewAttachment[] | null;
  state: "sending" | "sent" | "failed";
  error: string | null;
  /** How many of your messages in the conversation already read as this one when it was sent (`timesShown`). */
  seen: number;
};

/** How long a sent message stays on its way when the conversation never shows it as written (a reply that carried
 *  answers you tapped reads as one message with them, say), so it cannot linger. */
export const SENT_GRACE_MS = 4000;

/** How many of your messages read as `text`. A reply that carries held card answers is sent as those answers, a blank
 *  line, then your words, so a message ending in the text counts too. */
export function timesShown(mine: string[], text: string): number {
  const t = text.trim();
  if (!t) return 0;
  return mine.filter((m) => m.trim() === t || m.trim().endsWith(`\n\n${t}`)).length;
}

/** Whether a message still shows on its way: until the conversation shows it as yours (one more message reads as it
 *  than when it was sent). One that failed stays, with Retry, until you send it again or take it back. A message with
 *  files and no words has nothing to match, so it goes once it is sent. */
export function stillOnItsWay(o: Outgoing, mine: string[]): boolean {
  if (o.state === "failed") return true;
  if (!o.text.trim()) return o.state === "sending";
  return timesShown(mine, o.text) <= o.seen;
}

/** Your messages in a conversation as it shows them, oldest first: each turn's prompt (when you wrote it, not a
 *  colleague's report coming back or a check coming due) and what you sent into a turn while it worked. */
export function yourMessages(turns: LiveRun[]): string[] {
  return turns.flatMap((t) => [
    ...(t.run.prompt && t.run.kind.type !== "answered" && !t.run.check ? [t.run.prompt] : []),
    ...t.events.flatMap((ev) => (ev.type === "user" ? [ev.text] : [])),
  ]);
}

/** Events that are not the employee answering: the session starting, your messages and their being taken, stderr. */
const QUIET_EVENTS = new Set<RunEvent["type"]>(["init", "user", "heard", "stderr"]);

/** Whether a turn has begun answering: anything streamed or recorded, or it has ended. Until then the turn before it
 *  keeps its look, so sending does not restyle the conversation under you. */
export function hasBegun(t: LiveRun): boolean {
  return t.events.some((ev) => !QUIET_EVENTS.has(ev.type)) || !!t.live.trim() || !!t.run.endedAt;
}

/** The turn that leads the latest exchange: the last one that has begun answering, or the last of all when none has.
 *  Turns before it step back. */
export function leadTurn(turns: LiveRun[]): number {
  for (let i = turns.length - 1; i >= 0; i--) if (hasBegun(turns[i])) return i;
  return turns.length - 1;
}
