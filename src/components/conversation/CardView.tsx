import { useState, type ReactNode } from "react";
import { call, errorMessage } from "../../api/client";
import type { CardAnswer, CardItem, CardReply, EmployeeView, Run } from "../../api/types";
import { firstName } from "../../lib/format";
import { toast } from "../../lib/toasts";
import { Icon } from "../Icon";
import "../../screens/Inbox.css";

/** Sends your answer on `run`'s card and says what went. Resolves false, having said why, when it failed. */
export async function answerCard(run: Run, reply: CardReply, name: string): Promise<boolean> {
  let replies: Run[];
  try {
    replies = await call("answer_card", { runId: run.id, answer: reply });
  } catch (err) {
    toast(errorMessage(err), "err");
    return false;
  }
  const ask = run.card?.ask;
  const said =
    reply.type === "choice" ? `Sent to ${name}: ${ask?.type === "choices" ? ask.choices[reply.index] : ""}.`
    : reply.type === "items" ? `Sent to ${name}.`
    : reply.type === "done" ? (replies.length > 1 ? `Done. ${replies.length} runs carry on.` : `Done. ${name} carries on.`)
    : ask?.type === "chore" ? "Dismissed."
    : `Dismissed. ${name} goes with the safe default.`;
  toast(said, "ok");
  return true;
}

/** What became of an answered card, as one line. */
function answered(a: CardAnswer): string {
  switch (a.how) {
    case "tapped": return `Sent: ${a.text}`;
    case "dismissed": return "You dismissed it";
    case "replaced": return "A newer card replaced this one";
    case "typed": return "You answered in the thread";
  }
}

/** A run's card with the answer built in: buttons for a decision, a tick list, or Done for a chore. Once
 *  answered, one line on what you did. */
export function CardView({ run, e, onAnswer }: { run: Run; e: EmployeeView; onAnswer: (reply: CardReply) => Promise<void> }) {
  const [sending, setSending] = useState(false);
  const card = run.card;
  if (!card) return null;
  if (card.answer) return <p className="card-answered">{answered(card.answer)}</p>;
  const name = firstName(e.name);
  const ask = card.ask;
  const send = async (reply: CardReply) => {
    setSending(true);
    try { await onAnswer(reply); } finally { setSending(false); }
  };
  const dismiss = (
    <button className="btn btn-ghost btn-sm" title="They go with the safe default they stated" disabled={sending} onClick={() => void send({ type: "dismiss" })}>
      Dismiss
    </button>
  );
  return (
    <div onClick={(ev) => ev.stopPropagation()}>
      {ask.type === "choices" ? (
        <div className="card-choices">
          {ask.choices.map((c, i) => (
            <button key={i} className={`btn btn-sm ${i === ask.recommended ? "btn-primary" : ""}`} disabled={sending} onClick={() => void send({ type: "choice", index: i })}>
              {c}{i === ask.recommended ? <span className="card-rec">Recommended</span> : null}
            </button>
          ))}
          {dismiss}
        </div>
      ) : ask.type === "items" ? (
        <ItemsCard verb={ask.verb} items={ask.items} name={name} sending={sending} send={send} dismiss={dismiss} />
      ) : (
        <div className="card-choices">
          <button className="btn btn-primary btn-sm" title="You did it. Every run waiting on this carries on." disabled={sending} onClick={() => void send({ type: "done" })}>
            <Icon name="check" size={13} /> Done
          </button>
          <button className="btn btn-ghost btn-sm" disabled={sending} onClick={() => void send({ type: "dismiss" })}>Dismiss</button>
        </div>
      )}
    </div>
  );
}

/** A list to approve one by one: a tick and an optional note per item, then Send. */
function ItemsCard({ verb, items, name, sending, send, dismiss }: {
  verb: string;
  items: CardItem[];
  name: string;
  sending: boolean;
  send: (reply: CardReply) => Promise<void>;
  dismiss: ReactNode;
}) {
  const [ticks, setTicks] = useState<Record<string, boolean>>({});
  const [notes, setNotes] = useState<Record<string, string | undefined>>({});
  const ticked = items.filter((i) => ticks[i.id]).length;
  const ready = ticked > 0 || items.some((i) => notes[i.id]?.trim());
  const picks = items.map((i) => ({ id: i.id, tick: !!ticks[i.id], note: notes[i.id]?.trim() || null }));
  return (
    <div className="card-items">
      <ul>
        {items.map((i) => (
          <li key={i.id}>
            <div className="card-item">
              <label>
                <input
                  type="checkbox" checked={!!ticks[i.id]} disabled={sending} title={`${verb} ${i.id}`}
                  onChange={(ev) => setTicks((t) => ({ ...t, [i.id]: ev.target.checked }))}
                />
                <b>{i.id}</b>
                <span>{i.text}</span>
                {i.recommended ? <span className="card-rec">Recommended</span> : null}
              </label>
              {notes[i.id] === undefined ? (
                <button className="btn btn-ghost btn-sm" disabled={sending} onClick={() => setNotes((n) => ({ ...n, [i.id]: "" }))}>Note</button>
              ) : null}
            </div>
            {notes[i.id] !== undefined ? (
              <input
                className="input" autoFocus maxLength={300} placeholder={`Note for ${name} (optional)`} value={notes[i.id]} disabled={sending}
                onChange={(ev) => setNotes((n) => ({ ...n, [i.id]: ev.target.value }))}
              />
            ) : null}
          </li>
        ))}
      </ul>
      <div className="card-items-foot">
        <span className="faint">{ticked} of {items.length} ticked</span>
        <span className="grow" />
        {dismiss}
        <button className="btn btn-primary btn-sm" disabled={sending || !ready} onClick={() => void send({ type: "items", picks })}>Send</button>
      </div>
    </div>
  );
}
