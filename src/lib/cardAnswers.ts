import { call, errorMessage } from "../api/client";
import type { Card, CardReply, Run } from "../api/types";
import { whenInline } from "./format";
import { createStore, useStore } from "./store";
import { isOpen } from "./threads";
import { toast } from "./toasts";

/** How long an answer waits, with Undo, before it goes. */
export const UNDO_MS = 4000;

/** An answer you gave on a card (`index` in run `runId`'s cards), from when you give it until a little after it went.
 *  said: the answer in a few words, "Merge". going: it went, or is going, so Undo no longer takes it back. */
export type PendingAnswer = { runId: string; index: number; said: string; going: boolean };
const pending = createStore<PendingAnswer[]>([]);
/** Takes back each answer still waiting to go, by `runId:index`. */
const undos = new Map<string, () => void>();
export const onCard = (runId: string, index: number) => (p: PendingAnswer) => p.runId === runId && p.index === index;

/** Every answer on its way, oldest first. */
export const usePendingAnswers = () => useStore(pending);

/** An answer on the card at `index` of run `runId`'s cards is on its way. */
export const answering = (runId: string, index: number): boolean => pending.get().some(onCard(runId, index));

/** Takes back an answer that has not gone yet, so its card is open again. */
export function undoAnswer(runId: string, index: number) {
  undos.get(`${runId}:${index}`)?.();
}

/** Takes back the newest answer that has not gone yet. False when none is waiting to go. */
export function undoLatest(): boolean {
  const p = pending.get().filter((x) => !x.going).pop();
  if (p) undoAnswer(p.runId, p.index);
  return !!p;
}

/** Your answer on the card at `index` of `run`'s cards. It waits `UNDO_MS`, shown as on its way with Undo, then goes,
 *  and says what went. Resolves true once it went; false when you took it back, or when it failed, having said why. */
export async function answerCard(run: Run, index: number, reply: CardReply, name: string, opts: { quiet?: boolean } = {}): Promise<boolean> {
  const mine = onCard(run.id, index);
  if (pending.get().some(mine)) return false;
  const card = run.cards[index];
  const key = `${run.id}:${index}`;
  const forget = () => pending.set(pending.get().filter((p) => !mine(p)));
  pending.set([...pending.get(), { runId: run.id, index, said: saying(card, reply), going: false }]);
  const go = await new Promise<boolean>((done) => {
    const timer = window.setTimeout(() => done(true), UNDO_MS);
    undos.set(key, () => { window.clearTimeout(timer); done(false); });
  });
  undos.delete(key);
  if (!go) {
    forget();
    return false;
  }
  pending.set(pending.get().map((p) => (mine(p) ? { ...p, going: true } : p)));
  // The run holds this answer back while it asks something you have not answered, and hears them all at once.
  // Cards you answered since `run` was read are on their way, so they do not count, and news asks nothing.
  const rest = run.cards.filter((c, i) => i !== index && isOpen(c) && !answering(run.id, i)).length;
  let replies: Run[];
  try {
    replies = await call("answer_card", { runId: run.id, card: index, answer: reply });
  } catch (err) {
    forget();
    toast(errorMessage(err), "err");
    return false;
  }
  // Listed a little longer: runs read before it went still show the card open, so the card goes on showing it on its
  // way, and the run's other answers still count it as answered.
  window.setTimeout(forget, UNDO_MS + 1000);
  // News sends nothing, so there is nothing to say; a quiet answer leaves the saying to its caller (the Inbox's own toast).
  if (reply.type === "seen" || opts.quiet) return true;
  if (rest) {
    toast(`Noted. ${name} hears your answers once you answer ${rest === 1 ? "the other question" : `the other ${rest}`}.`, "ok");
    return true;
  }
  const ask = card?.ask;
  const to = replies.length > 1 ? `${name} and ${replies.length - 1} more` : name;
  const said =
    reply.type === "choice" ? `Sent to ${to}: ${ask?.type === "choices" ? ask.choices[reply.index] : ""}.`
    : reply.type === "items" || reply.type === "text" ? `Sent to ${to}.`
    : reply.type === "done" ? (replies.length > 1 ? `Done. ${replies.length} runs carry on.` : `Done. ${name} carries on.`)
    : reply.type === "cant" ? `Sent to ${to}: you can't do it.`
    : reply.type === "reject" ? `No. ${name} drops it.`
    : reply.type === "later" ? (card?.timesBack ? "Parked in Later. It won't come back by itself." : `Later. It comes back ${whenInline(reply.until)}.`)
    : ask?.type === "chore" ? "Not now."
    : `Not now. ${name} brings it back tomorrow.`;
  toast(said, "ok");
  return true;
}

/** The day a card you put off comes back, "Mon, 2 Nov". */
export const backDay = (iso: string) => new Date(iso).toLocaleDateString(undefined, { weekday: "short", day: "numeric", month: "short" });

/** Your answer in a few words, as the card shows it on its way: "Merge", "Post 2", "Later (Mon, 2 Nov)". */
function saying(card: Card | undefined, reply: CardReply): string {
  const ask = card?.ask;
  switch (reply.type) {
    case "choice": return ask?.type === "choices" ? ask.choices[reply.index] ?? "" : "";
    case "items": {
      const ticked = reply.picks.filter((p) => p.tick).length;
      return ticked && ask?.type === "items" ? `${ask.verb} ${ticked}` : "Notes";
    }
    case "done": return "I've done it";
    case "cant": return "Can't";
    case "seen": return "Got it";
    case "dismiss": return "Not now";
    case "reject": return ask?.type === "items" ? `Don't ${ask.verb.toLowerCase()}` : "No";
    case "later": return card?.timesBack ? "Later (parks it)" : `Later (${backDay(reply.until)})`;
    case "text":
    case "ask": return `"${reply.text}"`;
  }
}

/** A question about the card at `index` of run `runId`'s cards. It answers nothing, so it goes straight to them and the
 *  card stays where it is. Resolves whether it went, having said why when it did not. */
export async function askAboutCard(runId: string, index: number, name: string, text: string): Promise<boolean> {
  try {
    await call("answer_card", { runId, card: index, answer: { type: "ask", text } });
    toast(`Asked ${name}. The card stays open until you answer it.`, "ok");
    return true;
  } catch (err) {
    toast(errorMessage(err), "err");
    return false;
  }
}
