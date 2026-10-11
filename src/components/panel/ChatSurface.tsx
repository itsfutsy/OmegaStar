import { forwardRef, useEffect, useImperativeHandle, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { errorMessage } from "../../api/client";
import type { EmployeeView, NewAttachment } from "../../api/types";
import { firstName } from "../../lib/format";
import { toast } from "../../lib/toasts";
import type { LiveRun } from "../../lib/runs";
import { getDraft, setDraft } from "../../lib/drafts";
import { Icon } from "../Icon";
import { SubAgents } from "../SubAgents";
import { ConfirmButton, ErrorNote } from "../ui";
import "./ChatTab.css";

const MAX_FILES = 10;
const MAX_FILE_MB = 20;

type Pending = { id: number; file: File; preview: string | null };
let pendingN = 0;

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
  focus: () => void;
};

/**
 * The surface a conversation with an employee happens on: the scrolling transcript (`children`), a drop
 * zone for files, and the composer. The ongoing chat and every thread share it; what differs is where a
 * message goes (`onSend`) and what sits in the composer's bar (`actions`). While a turn is in flight the
 * composer stays open: what you send reaches the employee as they work.
 */
export const ChatSurface = forwardRef<ChatSurfaceHandle, {
  e: EmployeeView;
  /** A turn is in flight. */
  busy: boolean;
  /** The turn in flight, for its sub-agents and the Stop button. */
  live?: LiveRun;
  placeholder: string;
  /** Right of the bar while nothing is in flight. */
  hint?: ReactNode;
  draftKey: string;
  /** Changes as content streams, so the view stays pinned to the bottom unless the reader scrolled up. */
  scrollKey: string;
  onSend: (text: string, attachments: NewAttachment[]) => Promise<void>;
  /** A second way to send, "Send & done" (also Ctrl or ⌘ + Enter): the message goes and the conversation is put away. */
  onSendDone?: (text: string, attachments: NewAttachment[]) => Promise<void>;
  /** Sending starts a task, so the button says so. */
  startsTask?: boolean;
  onStop?: () => Promise<void>;
  /** Extra buttons in the bar, after the paperclip. */
  actions?: ReactNode;
  /** The composer takes focus when it appears. Off in an Inbox row, so its keys keep working until you click in. */
  autoFocus?: boolean;
  children: ReactNode;
}>(function ChatSurface({ e, busy, live, placeholder, hint, draftKey, scrollKey, onSend, onSendDone, startsTask = false, onStop, actions, autoFocus = true, children }, ref) {
  const [text, setText] = useState(() => getDraft(draftKey));
  const [sending, setSending] = useState(false);
  const [sendError, setSendError] = useState<string | null>(null);
  const [files, setFiles] = useState<Pending[]>([]);
  const [dragging, setDragging] = useState(false);
  const picker = useRef<HTMLInputElement>(null);
  const scroller = useRef<HTMLDivElement>(null);
  const input = useRef<HTMLTextAreaElement>(null);
  const stick = useRef(true);
  const name = firstName(e.name);
  const hasContent = !!text.trim() || files.length > 0;

  useImperativeHandle(ref, () => ({
    prefill: (t) => {
      setText(t);
      requestAnimationFrame(() => {
        const el = input.current;
        if (el) { el.focus(); el.setSelectionRange(el.value.length, el.value.length); }
      });
    },
    focus: () => input.current?.focus(),
  }), []);

  useEffect(() => { setDraft(draftKey, text); }, [draftKey, text]);

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

  // Keep pinned to the bottom while content streams, unless the reader scrolled up.
  useLayoutEffect(() => {
    const el = scroller.current;
    if (el && stick.current) el.scrollTop = el.scrollHeight;
  }, [scrollKey]);

  useLayoutEffect(() => {
    const el = input.current;
    if (!el) return;
    el.style.height = "0px";
    el.style.height = `${Math.min(200, Math.max(44, el.scrollHeight))}px`;
  }, [text]);

  const wasSending = useRef(false);
  useEffect(() => {
    if (!sending && (autoFocus || wasSending.current)) input.current?.focus({ preventScroll: true });
    wasSending.current = sending;
  }, [sending, autoFocus]);

  const send = async (to = onSend) => {
    const body = text.trim();
    if ((!body && !files.length) || sending) return;
    setSending(true);
    setSendError(null);
    stick.current = true;
    try {
      const attachments = await Promise.all(files.map((f) => toAttachment(f.file)));
      await to(body, attachments);
      // Cleared in the store too: sending may have moved you to another conversation already.
      setDraft(draftKey, "");
      setText("");
      clearFiles();
    } catch (err) {
      setSendError(errorMessage(err));
    } finally {
      setSending(false);
    }
  };

  return (
    <div
      className={`chat ${dragging ? "chat-dragging" : ""}`}
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
      <div className="chat-scroll" ref={scroller} onScroll={(ev) => {
        const el = ev.currentTarget;
        stick.current = el.scrollHeight - el.scrollTop - el.clientHeight < 80;
      }}>
        {children}
      </div>
      <div className="composer-wrap">
        {sendError ? <ErrorNote text={sendError} /> : null}
        {live ? <SubAgents tasks={live.tasks} runActive /> : null}
        <div className={`composer ${sending ? "composer-busy" : ""}`}>
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
          <textarea
            ref={input} className="composer-input" value={text} disabled={sending} rows={1}
            placeholder={busy ? `Message ${name}. They'll see it as they work.` : placeholder}
            onChange={(ev) => setText(ev.target.value)}
            onPaste={(ev) => {
              const pasted = [...ev.clipboardData.files];
              if (pasted.length) { ev.preventDefault(); addFiles(pasted); }
            }}
            onKeyDown={(ev) => {
              if (ev.key === "Enter" && !ev.shiftKey && !ev.nativeEvent.isComposing) { ev.preventDefault(); void send(onSendDone && (ev.metaKey || ev.ctrlKey) ? onSendDone : onSend); }
            }}
            aria-label={placeholder}
          />
          <div className="composer-bar">
            <input ref={picker} type="file" multiple hidden onChange={(ev) => { addFiles([...(ev.target.files ?? [])]); ev.target.value = ""; }} />
            <button className="btn btn-ghost btn-icon btn-sm" aria-label="Attach files" title="Attach files, or paste a screenshot" onClick={() => picker.current?.click()} disabled={sending}><Icon name="paperclip" size={14} /></button>
            {actions}
            <span className="grow" />
            {live && onStop ? (
              <ConfirmButton label="Stop" icon="stop" className="btn btn-ghost" size="btn-sm" question={`Stop ${name}?`} confirmLabel="Stop it" onConfirm={onStop} />
            ) : (
              hint ? <span className="composer-hint faint">{hint}</span> : null
            )}
            {startsTask && hasContent ? (
              <button className="btn btn-primary btn-sm composer-send composer-send-task" title="Start as a new task (Enter). Shift+Enter adds a new line." disabled={sending} onClick={() => void send()}>
                <Icon name="branch" size={13} /> Start task
              </button>
            ) : (
              <>
                {onSendDone && hasContent ? (
                  <button className="btn btn-sm" title="Send, and bring it back only if the answer needs you (Ctrl+Enter)" disabled={sending} onClick={() => void send(onSendDone)}>
                    Send &amp; done
                  </button>
                ) : null}
                <button className="btn btn-primary btn-icon btn-sm composer-send" aria-label="Send" title="Send (Enter). Shift+Enter adds a new line." disabled={sending || !hasContent} onClick={() => void send()}>
                  <Icon name="send" size={14} strokeWidth={1.8} />
                </button>
              </>
            )}
          </div>
        </div>
      </div>
    </div>
  );
});
