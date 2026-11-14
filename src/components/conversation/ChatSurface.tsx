import { forwardRef, useCallback, useEffect, useImperativeHandle, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { errorMessage } from "../../api/client";
import type { EmployeeView, NewAttachment } from "../../api/types";
import { firstName, MOD } from "../../lib/format";
import { toast } from "../../lib/toasts";
import type { LiveRun } from "../../lib/runs";
import { useComposerAsks } from "../../lib/composer";
import { getDraft, setDraft } from "../../lib/drafts";
import { SENT_GRACE_MS, stillOnItsWay, timesShown, type Outgoing } from "../../lib/outbox";
import { ms, reducedMotion } from "../../lib/motion";
import { Icon } from "../Icon";
import { ConfirmButton, ErrorNote } from "../ui";
import type { AimTarget } from "./CardView";
import "./Chat.css";

const MAX_FILES = 10;
const NONE: string[] = [];
const MAX_FILE_MB = 20;

type Pending = { id: number; file: File; preview: string | null };
let pendingN = 0;
let outgoingN = 0;
/** Where a message on its way goes, and its files as you picked them, kept for Retry and Edit. */
type Route = { to: (text: string, attachments: NewAttachment[]) => Promise<void>; files: File[] };

const isImage = (name: string) => /\.(png|jpe?g|gif|webp|bmp|heic)$/i.test(name);

function toAttachment(file: File): Promise<NewAttachment> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve({ name: file.name, data: String(reader.result).split(",", 2)[1] ?? "" });
    reader.onerror = () => reject(new Error(`Could not read ${file.name}.`));
    reader.readAsDataURL(file);
  });
}

export type ChatSurfaceHandle = {
  /** Puts text in the composer and moves the caret to its end. */
  prefill: (text: string) => void;
  /** Aims the composer at a card, so what you write answers it (null: back to the conversation). */
  aim: (target: AimTarget | null) => void;
};

/**
 * The surface a conversation with an employee happens on: the scrolling transcript (`children`), a drop
 * zone for files, and the composer. Every conversation shares it; what differs is where a message goes
 * (`onSend`). The composer is the field and one button: Send, or Stop while a turn is in flight and the field is
 * empty. While a turn is in flight the composer stays open: what you send reaches the employee as they work. Files come
 * in by dropping or pasting them, or from the conversation's menu (`useComposerAsks`).
 */
export const ChatSurface = forwardRef<ChatSurfaceHandle, {
  e: EmployeeView;
  /** The turn in flight, if any, for the placeholder and the Stop button. */
  live?: LiveRun;
  placeholder: string;
  /** Said above the composer once you start typing: "This won't answer the card above." */
  cardNote?: string | null;
  draftKey: string;
  /** Changes as content streams, so the view stays pinned to the bottom unless the reader scrolled up. */
  scrollKey: string;
  onSend: (text: string, attachments: NewAttachment[]) => Promise<void>;
  /** A second way to send, "Send & done" (Ctrl or ⌘ + Enter): the message goes and the conversation is put away. */
  onSendDone?: (text: string, attachments: NewAttachment[]) => Promise<void>;
  onStop?: () => Promise<void>;
  /** The composer takes focus when it appears. Off in an Inbox row, so its keys keep working until you click in. */
  autoFocus?: boolean;
  /** The card the composer is aimed at changed (its key, or null). */
  onAim?: (key: string | null) => void;
  /** Open at the end of the conversation and keep to it (a thread); off for a page read from the top (a new one). */
  pin?: boolean;
  /** Your messages as the conversation shows them (`yourMessages`), so a message on its way gives way to the real one. */
  mine?: string[];
  children: ReactNode;
}>(function ChatSurface({ e, live, placeholder, cardNote, draftKey, scrollKey, onSend, onSendDone, onStop, autoFocus = true, onAim, pin = true, mine = NONE, children }, ref) {
  const [text, setText] = useState(() => getDraft(draftKey));
  const [sending, setSending] = useState(false);
  const [sendError, setSendError] = useState<string | null>(null);
  // What you sent that the conversation does not show yet: on its way, or kept with Retry when it did not go.
  const [outbox, setOutbox] = useState<Outgoing[]>([]);
  const routes = useRef(new Map<number, Route>());
  const mineRef = useRef(mine);
  mineRef.current = mine;
  const textRef = useRef(text);
  textRef.current = text;
  // Set by Send: the next scroll goes to your message however far it is, easing rather than jumping.
  const ownSend = useRef(false);
  const [files, setFiles] = useState<Pending[]>([]);
  const [dragging, setDragging] = useState(false);
  const picker = useRef<HTMLInputElement>(null);
  const scroller = useRef<HTMLDivElement>(null);
  const input = useRef<HTMLTextAreaElement>(null);
  const stick = useRef(pin);
  // Until you scroll, the opening view lands on whole lines: a spacer at the end takes up the part of a line the top
  // edge would cut, so the top line is either whole or gone.
  const touched = useRef(false);
  const spacer = useRef<HTMLDivElement>(null);
  const landWhole = useCallback(() => {
    const el = scroller.current, sp = spacer.current;
    if (!el || !sp || touched.current) return;
    sp.style.height = "0px";
    el.scrollTop = el.scrollHeight;
    const top = el.getBoundingClientRect().top;
    const walk = document.createTreeWalker(el, NodeFilter.SHOW_TEXT);
    const range = document.createRange();
    for (let n = walk.nextNode(); n; n = walk.nextNode()) {
      if (!n.textContent?.trim()) continue;
      range.selectNodeContents(n);
      const box = range.getBoundingClientRect();
      if (box.bottom <= top || box.top >= top + 2) { if (box.top >= top + 2) break; continue; }
      for (const line of range.getClientRects()) {
        if (line.top < top - 0.5 && line.bottom > top + 0.5) {
          const cut = Math.ceil(line.bottom - top);
          if (cut > 0 && cut <= 48) { sp.style.height = `${cut}px`; el.scrollTop = el.scrollHeight; }
          return;
        }
      }
    }
  }, []);
  const name = firstName(e.name);
  const hasContent = !!text.trim() || files.length > 0;
  // The composer aimed at a card: what you write answers that card.
  const [aim, setAimState] = useState<AimTarget | null>(null);
  const onAimRef = useRef(onAim);
  onAimRef.current = onAim;
  const setAim = useCallback((t: AimTarget | null) => {
    setAimState(t);
    onAimRef.current?.(t?.key ?? null);
    if (t) requestAnimationFrame(() => input.current?.focus());
  }, []);

  // A turn takes messages until its result; after that a message waits to be its next turn. A turn the
  // usage limit sent back to the queue starts again at a new init, so only a result after it counts.
  const attempt = live ? live.events.slice(live.events.map((ev) => ev.type).lastIndexOf("init") + 1) : [];
  const steerable = live?.run.state === "running" && !attempt.some((ev) => ev.type === "result");

  const prefill = useCallback((t: string) => {
    setText(t);
    requestAnimationFrame(() => {
      const el = input.current;
      if (el) { el.focus(); el.setSelectionRange(el.value.length, el.value.length); }
    });
  }, []);
  useImperativeHandle(ref, () => ({ prefill, aim: setAim }), [prefill, setAim]);

  useEffect(() => { setDraft(draftKey, text); }, [draftKey, text]);

  // The conversation's menu: words to start from, or the file picker.
  useComposerAsks(draftKey, (ask) => {
    if (ask.type === "attach") picker.current?.click();
    else prefill(ask.text);
  });

  // Previews are object URLs: release them when they leave the composer.
  const previews = useRef(new Set<string>());
  useEffect(() => () => previews.current.forEach((u) => URL.revokeObjectURL(u)), []);
  const clearFiles = (keep: Pending[] = []) => {
    for (const f of files) if (f.preview && !keep.includes(f)) { URL.revokeObjectURL(f.preview); previews.current.delete(f.preview); }
    setFiles(keep);
  };

  const addFiles = (list: File[]) => {
    if (sending || list.length === 0) return;
    const tooBig = list.filter((f) => f.size > MAX_FILE_MB * 1024 * 1024);
    if (tooBig.length) toast(`${tooBig.map((f) => f.name).join(", ")} ${tooBig.length === 1 ? "is" : "are"} over ${MAX_FILE_MB} MB.`, "err");
    const room = MAX_FILES - files.length;
    const ok = list.filter((f) => !tooBig.includes(f));
    if (ok.length > room) toast(`Attach up to ${MAX_FILES} files at a time.`, "err");
    const added = ok.slice(0, Math.max(0, room)).map((file) => {
      const preview = file.type.startsWith("image/") ? URL.createObjectURL(file) : null;
      if (preview) previews.current.add(preview);
      return { id: ++pendingN, file, preview };
    });
    if (added.length) setFiles((cur) => [...cur, ...added]);
    input.current?.focus();
  };

  // The field fits its words. Sized before the pinning below measures, so a field that shrinks as Send clears it is
  // already its new height when the talk scrolls to your message.
  useLayoutEffect(() => {
    const el = input.current;
    if (!el) return;
    el.style.height = "0px";
    el.style.height = `${Math.min(200, Math.max(44, el.scrollHeight))}px`;
  }, [text]);

  // Keep pinned to the bottom while content streams, unless the reader scrolled up. A new line eases into view rather
  // than jumping the thread; opening a conversation, or a big change, is simply there.
  const settled = useRef(false);
  const easing = useRef<{ frame: number; from: number; at: number } | null>(null);
  // Your own message always eases into view, however far: one scroll, after the field has cleared and the message is in.
  useLayoutEffect(() => {
    const el = scroller.current;
    const own = ownSend.current;
    ownSend.current = false;
    if (el && own && !pin) { el.scrollTo({ top: el.scrollHeight, behavior: reducedMotion() ? "auto" : "smooth" }); return; }
    if (!el || !stick.current || !pin) return;
    const delta = el.scrollHeight - el.clientHeight - el.scrollTop;
    if (delta <= 0) { settled.current = true; return; }
    if (!settled.current || (delta >= 160 && !own) || reducedMotion()) {
      el.scrollTop = el.scrollHeight;
      if (!settled.current) landWhole();
      settled.current = true;
      return;
    }
    // Ease toward the bottom over --d-arrive; a line that arrives mid-ease retargets from where it is.
    if (easing.current) cancelAnimationFrame(easing.current.frame);
    const from = el.scrollTop;
    const at = performance.now();
    const dur = ms("--d-arrive") || 320;
    const step = (now: number) => {
      const t = Math.min(1, (now - at) / dur);
      const k = 1 - Math.pow(1 - t, 3);
      const target = el.scrollHeight - el.clientHeight;
      el.scrollTop = from + (target - from) * k;
      if (t < 1) easing.current = { frame: requestAnimationFrame(step), from, at };
      else { easing.current = null; landWhole(); }
    };
    easing.current = { frame: requestAnimationFrame(step), from, at };
  }, [scrollKey, outbox.length, landWhole, pin]);
  useEffect(() => () => { if (easing.current) cancelAnimationFrame(easing.current.frame); }, []);
  // What loads after the conversation opens (a draft, pictures, a page's height) keeps the end in view, so the last
  // question never opens cut off under the composer.
  useLayoutEffect(() => {
    const el = scroller.current, inner = el?.firstElementChild;
    if (!el || !inner) return;
    const ro = new ResizeObserver(() => {
      if (!stick.current || easing.current) return;
      if (touched.current) { el.scrollTop = el.scrollHeight; return; }
      landWhole();
    });
    ro.observe(inner);
    return () => ro.disconnect();
  }, [landWhole]);

  useEffect(() => {
    if (autoFocus) input.current?.focus({ preventScroll: true });
  }, [autoFocus]);

  const send = async (to = onSend) => {
    const body = text.trim();
    if (aim) {
      if (!body || sending) return;
      // The words go to the card at once: the composer clears and the card shows them on their way, with Undo. If they
      // do not go (Undo, or a failure), they come back here, aimed at the same card.
      const target = aim;
      setSendError(null);
      setDraft(draftKey, "");
      setText("");
      setAim(null);
      try {
        if (!(await target.send(body))) {
          setText((cur) => (cur.trim() ? cur : body));
          setAim(target);
        }
      } catch (err) {
        setSendError(errorMessage(err));
        setText((cur) => (cur.trim() ? cur : body));
        setAim(target);
      }
      return;
    }
    if ((!body && !files.length) || sending) return;
    // Your message shows at once, on its way, and the field clears in the same step; the conversation's own copy
    // replaces it when the call returns. Cleared in the store too: sending may move you to another conversation.
    const route: Route = { to, files: files.map((f) => f.file) };
    const o: Outgoing = { id: ++outgoingN, text: body, files: route.files.map((f) => f.name), attachments: null, state: "sending", error: null, seen: timesShown(mineRef.current, body) };
    routes.current.set(o.id, route);
    setSendError(null);
    stick.current = true;
    ownSend.current = true;
    setDraft(draftKey, "");
    setText("");
    clearFiles();
    setOutbox((cur) => [...cur, o]);
    await deliver(o, route);
  };

  /** Sends a message on its way. Sent, it waits for the conversation to show it (or SENT_GRACE_MS); failed, it stays
   *  with the reason and Retry, and its words are kept as the draft when the field is empty, so leaving loses nothing. */
  const deliver = async (o: Outgoing, route: Route) => {
    const mark = (patch: Partial<Outgoing>) => setOutbox((cur) => cur.map((x) => (x.id === o.id ? { ...x, ...patch } : x)));
    setSending(true);
    let attachments = o.attachments;
    try {
      attachments ??= await Promise.all(route.files.map(toAttachment));
      await route.to(o.text, attachments);
      mark({ attachments, state: "sent" });
      if (!textRef.current.trim() && getDraft(draftKey) === o.text) setDraft(draftKey, "");
      window.setTimeout(() => {
        setOutbox((cur) => cur.filter((x) => x.id !== o.id || x.state !== "sent"));
        routes.current.delete(o.id);
      }, SENT_GRACE_MS);
    } catch (err) {
      mark({ attachments, state: "failed", error: errorMessage(err) });
      if (!textRef.current.trim()) setDraft(draftKey, o.text);
    } finally {
      setSending(false);
    }
  };
  const retry = (o: Outgoing) => {
    const route = routes.current.get(o.id);
    if (!route || sending) return;
    const again: Outgoing = { ...o, state: "sending", error: null, seen: timesShown(mineRef.current, o.text) };
    setOutbox((cur) => cur.map((x) => (x.id === o.id ? again : x)));
    stick.current = true;
    void deliver(again, route);
  };
  /** Takes a failed message back into the field (and its files), to change before sending again. */
  const edit = (o: Outgoing) => {
    const route = routes.current.get(o.id);
    setOutbox((cur) => cur.filter((x) => x.id !== o.id));
    routes.current.delete(o.id);
    if (!textRef.current.trim()) prefill(o.text);
    if (route?.files.length) addFiles(route.files);
  };
  const onItsWay = outbox.filter((o) => stillOnItsWay(o, mine));

  return (
    <div className="chat">
    <div
      className={`chat-talk ${dragging ? "chat-dragging" : ""}`}
      onDragOver={(ev) => {
        if (!ev.dataTransfer.types.includes("Files") || sending) return;
        ev.preventDefault();
        setDragging(true);
      }}
      onDragLeave={(ev) => { if (!ev.currentTarget.contains(ev.relatedTarget as Node | null)) setDragging(false); }}
      onDrop={(ev) => {
        ev.preventDefault();
        setDragging(false);
        addFiles([...ev.dataTransfer.files]);
      }}
    >
      {dragging ? <div className="chat-drop"><Icon name="paperclip" size={18} /> Drop to attach for {name}</div> : null}
      <div className="chat-scroll" ref={scroller}
        onWheel={() => { touched.current = true; }} onTouchMove={() => { touched.current = true; }} onKeyDown={() => { touched.current = true; }}
        onScroll={(ev) => {
          const el = ev.currentTarget;
          if (!pin) return;
          stick.current = el.scrollHeight - el.scrollTop - el.clientHeight < 160;
        }}>
        <div className="chat-scroll-inner">
          {children}
          {onItsWay.length ? (
            <div className="chat-inner chat-outbox">
              {onItsWay.map((o) => (
                <div key={o.id} className={`msg-wait ${o.state === "failed" ? "msg-out-failed" : ""}`}>
                  {o.files.length ? (
                    <div className="msg-files">
                      {o.files.map((n, i) => <span key={i} className="attach attach-sent"><Icon name={isImage(n) ? "image" : "file"} size={13} /><span className="attach-name">{n}</span></span>)}
                    </div>
                  ) : null}
                  {o.text ? <div className="msg-me">{o.text}</div> : null}
                  {o.state === "failed" ? (
                    <div className="msg-out-error" role="alert">
                      <span className="msg-wait-note"><Icon name="alert" size={13} /> Didn't send{o.error ? `: ${o.error}` : "."}</span>
                      <span className="msg-out-actions">
                        <button className="btn btn-ghost btn-sm" onClick={() => edit(o)}>Edit</button>
                        <button className="btn btn-sm" disabled={sending} onClick={() => retry(o)}><Icon name="refresh" size={13} /> Retry</button>
                      </span>
                    </div>
                  ) : <span className="msg-wait-note">Sending…</span>}
                </div>
              ))}
            </div>
          ) : null}
          <div ref={spacer} className="chat-land" aria-hidden />
        </div>
      </div>
      <div className="composer-wrap">
        {sendError ? <ErrorNote text={sendError} /> : null}
        {cardNote && text.trim() && !aim ? <p className="composer-note enter-fade" role="status"><Icon name="alert" size={13} /> {cardNote}</p> : null}
        <div className="composer-stack">
        {aim ? (
          <div className="composer-aim enter-fade">
            <Icon name="reply" size={14} />
            <span className="composer-aim-what"><span className="muted">Answering</span> <b>{aim.label}</b></span>
            <button type="button" className="btn btn-ghost btn-sm" disabled={sending || !text.trim()} data-tip="Sends it as a question. The card stays open."
              onClick={async () => { const t = text.trim(); if (t && await aim.ask(t)) { setText(""); setDraft(draftKey, ""); } }}>Ask instead</button>
            <button type="button" className="btn btn-ghost btn-sm" disabled={sending} data-tip={`${aim.name} sums it up in three short lines. The card stays open.`} onClick={aim.summarise}>Summarise</button>
            <button type="button" className="btn btn-ghost btn-icon btn-sm" aria-label="Back to the conversation" data-tip="Back to the conversation (Esc)" onClick={() => setAim(null)}><Icon name="close" size={14} /></button>
          </div>
        ) : null}
        <div className="composer">
          {files.length ? (
            <div className="composer-files">
              {files.map((f) => (
                <div key={f.id} className={`attach ${f.preview ? "attach-image" : ""}`} title={f.file.name}>
                  {f.preview ? <img src={f.preview} alt={f.file.name} /> : <><Icon name="file" size={13} /><span className="attach-name">{f.file.name}</span></>}
                  <button className="attach-remove" aria-label={`Remove ${f.file.name}`} disabled={sending} onClick={() => clearFiles(files.filter((x) => x !== f))}>
                    <Icon name="close" size={10} strokeWidth={2} />
                  </button>
                </div>
              ))}
            </div>
          ) : null}
          <div className="composer-row">
            <textarea
              ref={input} className="composer-input" value={text} rows={1}
              placeholder={aim ? `Your answer for ${aim.name}` : !live ? placeholder : steerable ? `Message ${name}. They'll see it as they work.` : `Message ${name}. It goes to them as their next turn.`}
              onChange={(ev) => setText(ev.target.value)}
              onPaste={(ev) => {
                const pasted = [...ev.clipboardData.files];
                if (pasted.length) { ev.preventDefault(); addFiles(pasted); }
              }}
              onKeyDown={(ev) => {
                if (ev.key === "Escape" && aim) { ev.preventDefault(); ev.stopPropagation(); setAim(null); return; }
                if (ev.key === "Enter" && !ev.shiftKey && !ev.nativeEvent.isComposing) { ev.preventDefault(); void send(onSendDone && !aim && (ev.metaKey || ev.ctrlKey) ? onSendDone : onSend); }
              }}
              aria-label={placeholder}
            />
            <input ref={picker} type="file" multiple hidden onChange={(ev) => { addFiles([...(ev.target.files ?? [])]); ev.target.value = ""; }} />
            <div className="composer-bar">
              {live && onStop && !hasContent && !aim ? (
                <ConfirmButton label="" ariaLabel={`Stop ${name}`} tip={`Stop ${name}`} icon="stop" className="btn btn-icon composer-send composer-stop" size="btn-sm" question={`Stop ${name}?`} confirmLabel="Stop it" onConfirm={onStop} />
              ) : (
                <button
                  className={`btn btn-icon btn-sm composer-send ${hasContent ? "composer-send-ready" : ""}`} aria-label={aim ? "Send your answer" : "Send"} disabled={sending || !hasContent} onClick={() => void send()}
                  data-tip={aim ? "Answers the card (Enter)" : onSendDone ? `Send (Enter). ${MOD} Enter sends and puts it away until the answer needs you.` : "Send (Enter). Shift+Enter adds a new line."}
                >
                  <Icon name="send" size={15} strokeWidth={1.75} />
                </button>
              )}
            </div>
          </div>
        </div>
        </div>
      </div>
    </div>
    </div>
  );
});
