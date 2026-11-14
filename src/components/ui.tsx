import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import type { Presence } from "../api/types";
import { initials } from "../lib/format";
import { ms, rollAnimations } from "../lib/motion";
import { useScrollFade } from "../lib/scrollFade";
import { dismissToast, useToasts, type Toast } from "../lib/toasts";
import { Icon, type IconName } from "./Icon";
import "./ui.css";

// ---------------------------------------------------------------- Avatar

/** A named avatar in its own colour. People are ringed, a proposed hire is dashed, you are the ink disc. An employee
 *  with a display image shows it in place of their initials. Presence is a still dot on the avatar (ringed in
 *  `--av-ring`, the surface it sits on): only where there is no room for a word. */
/** Whether an element's text is cut off: by an ellipsis on its one line, or by a clamp of a few. */
export const isCut = (el: HTMLElement) => el.scrollWidth > el.clientWidth + 1 || el.scrollHeight > el.clientHeight + 1;

/** For `onPointerEnter`: the element's whole text as its tooltip, only while it is cut off, so a long title can still
 *  be read in full. */
export function tipWhenCut(e: React.PointerEvent<HTMLElement>) {
  const el = e.currentTarget;
  if (isCut(el)) el.title = el.textContent ?? "";
  else el.removeAttribute("title");
}

export function Avatar({ name, hue, image, size = 36, presence, person = false, proposed = false, you = false, className }: {
  name: string; hue: number; image?: string | null; size?: number; presence?: Presence; person?: boolean; proposed?: boolean; you?: boolean; className?: string;
}) {
  const style = { "--h": hue, width: size, height: size, fontSize: Math.max(8, Math.round(size * 0.37)) } as React.CSSProperties;
  const kind = you ? "av-you" : proposed ? "av-proposed" : person ? "av-person" : "";
  return (
    <span className={`av ${kind} ${className ?? ""}`} style={style} aria-hidden>
      {image ? <img className="av-img" src={image} alt="" draggable={false} /> : <span className="av-i">{initials(name)}</span>}
      {presence && presence !== "idle" ? <span className={`av-dot av-dot-${presence}`} /> : null}
    </span>
  );
}

// ---------------------------------------------------------------- Slide-over

/** A surface that slides in from the window's right edge. Closing returns focus to `returnFocus` when given (pass the
 *  button that opened it: WebKit does not focus a button on click), else to whatever had focus when it opened. */
export function SlideOver({ open, onClose, width = 760, children, label, returnFocus }: {
  open: boolean; onClose: () => void; width?: number | string; children: ReactNode; label: string; returnFocus?: HTMLElement | null;
}) {
  const [mounted, setMounted] = useState(open);
  const [shown, setShown] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  const lastFocus = useRef<Element | null>(null);

  useEffect(() => {
    if (open) {
      lastFocus.current = document.activeElement;
      setMounted(true);
      const t = window.setTimeout(() => setShown(true), 20);
      return () => window.clearTimeout(t);
    }
    setShown(false);
    const t = window.setTimeout(() => setMounted(false), 260);
    return () => window.clearTimeout(t);
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !e.defaultPrevented) {
        const t = e.target as HTMLElement | null;
        if (t?.closest("[data-esc-stop]")) return;
        e.preventDefault();
        onClose();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  useEffect(() => {
    if (shown && ref.current && !ref.current.contains(document.activeElement)) {
      const first = ref.current.querySelector<HTMLElement>("[data-autofocus]") ?? ref.current;
      first.focus({ preventScroll: true });
    }
  }, [shown]);

  useEffect(() => {
    if (open) return;
    const to = returnFocus?.isConnected ? returnFocus : lastFocus.current;
    if (to instanceof HTMLElement) to.focus({ preventScroll: true });
  }, [open, returnFocus]);

  if (!mounted) return null;
  return (
    <>
      <div className={`scrim ${shown ? "scrim-on" : ""}`} onClick={onClose} />
      <aside
        ref={ref} tabIndex={-1} role="dialog" aria-label={label} aria-modal
        className={`slideover ${shown ? "slideover-on" : ""}`} style={{ width }}
      >
        {children}
      </aside>
    </>
  );
}

/** A slide-over's title, a line under it, and its close button. */
export function SheetHead({ title, lede, onClose }: { title: ReactNode; lede: ReactNode; onClose: () => void }) {
  return (
    <header className="sheet-head">
      <div className="stack">
        <h2 className="h-lg">{title}</h2>
        <p className="muted">{lede}</p>
      </div>
      <button className="btn btn-ghost btn-icon" aria-label="Close" onClick={onClose}><Icon name="close" /></button>
    </header>
  );
}

// ---------------------------------------------------------------- Inline confirm

export function ConfirmButton({ label, confirmLabel, question, onConfirm, icon, className, size = "", ariaLabel, tip }: {
  label: string; confirmLabel: string; question: string; onConfirm: () => void | Promise<void>; icon?: IconName; className: string; size?: "" | "btn-sm";
  /** For an icon-only button: its name, and its tooltip. */
  ariaLabel?: string; tip?: string;
}) {
  const [asking, setAsking] = useState(false);
  const [busy, setBusy] = useState(false);
  const yes = useRef<HTMLButtonElement>(null);
  useEffect(() => { if (asking) yes.current?.focus(); }, [asking]);
  if (!asking) {
    return (
      <button type="button" className={`${className} ${size}`} aria-label={ariaLabel} data-tip={tip} onClick={() => setAsking(true)}>
        {icon ? <Icon name={icon} size={14} /> : null}
        {label}
      </button>
    );
  }
  return (
    <span className="confirm" data-esc-stop onKeyDown={(e) => { if (e.key === "Escape") { e.stopPropagation(); setAsking(false); } }}>
      <span className="confirm-q">{question}</span>
      <button type="button" className={`btn btn-ghost ${size}`} onClick={() => setAsking(false)} disabled={busy}>Keep</button>
      <button
        ref={yes} type="button" className={`btn btn-danger-solid ${size}`} disabled={busy}
        onClick={async () => { setBusy(true); try { await onConfirm(); } finally { setBusy(false); setAsking(false); } }}
      >
        {confirmLabel}
      </button>
    </span>
  );
}

// ---------------------------------------------------------------- Popover

export function Popover({ anchor, open, onClose, children, align = "end", width = 320 }: {
  anchor: React.RefObject<HTMLElement | null>; open: boolean; onClose: () => void; children: ReactNode; align?: "start" | "end"; width?: number;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState<{ top: number; left: number } | null>(null);

  useLayoutEffect(() => {
    if (!open || !anchor.current) return;
    const r = anchor.current.getBoundingClientRect();
    const left = align === "end" ? Math.max(8, r.right - width) : Math.min(window.innerWidth - width - 8, r.left);
    setPos({ top: r.bottom + 6, left });
  }, [open, anchor, align, width]);

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      const t = e.target as Node;
      if (ref.current?.contains(t) || anchor.current?.contains(t)) return;
      onClose();
    };
    const onKey = (e: KeyboardEvent) => { if (e.key === "Escape") { e.preventDefault(); e.stopPropagation(); onClose(); anchor.current?.focus(); } };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey, true);
    return () => { document.removeEventListener("mousedown", onDown); document.removeEventListener("keydown", onKey, true); };
  }, [open, onClose, anchor]);

  if (!open || !pos) return null;
  return createPortal(
    <div ref={ref} className="popover" style={{ top: pos.top, left: pos.left, width }} role="dialog">
      {children}
    </div>,
    document.body,
  );
}

// ---------------------------------------------------------------- Overlay

/** Something seen full-window (an image, a page) on a dark scrim. Escape, the close button or a click on the scrim
 *  closes it; `onKey` gets the other keys first. */
export function Overlay({ label, onClose, onKey, children }: {
  label: string; onClose: () => void; onKey?: (e: KeyboardEvent) => void; children: ReactNode;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const latest = useRef({ onClose, onKey });
  latest.current = { onClose, onKey };
  useEffect(() => {
    const last = document.activeElement;
    ref.current?.focus({ preventScroll: true });
    const key = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return latest.current.onKey?.(e);
      e.preventDefault();
      e.stopPropagation();
      latest.current.onClose();
    };
    document.addEventListener("keydown", key, true);
    return () => {
      document.removeEventListener("keydown", key, true);
      if (last instanceof HTMLElement) last.focus({ preventScroll: true });
    };
  }, []);
  return createPortal(
    <div ref={ref} className="overlay" role="dialog" aria-modal aria-label={label} tabIndex={-1} onClick={(e) => { if (e.target === e.currentTarget) onClose(); }}>
      {children}
      <button className="overlay-close" aria-label="Close" onClick={onClose}><Icon name="close" size={16} /></button>
    </div>,
    document.body,
  );
}

// ---------------------------------------------------------------- Toasts

/** How long a leaving toast stays mounted while it fades (--d-ui, or --d-fade under reduced motion, both shorter). */
const TOAST_LEAVE_MS = 160;

/** Toasts land from just above, one line each, and fade where they are when they go: nothing pops. */
export function Toasts() {
  const toasts = useToasts();
  const [leaving, setLeaving] = useState<Toast[]>([]);
  const shown = useRef<Toast[]>([]);
  // One timer per group that leaves; a later change never cancels an earlier group's removal.
  const timers = useRef<number[]>([]);
  useEffect(() => {
    const gone = shown.current.filter((t) => !toasts.some((x) => x.id === t.id));
    shown.current = toasts;
    if (!gone.length) return;
    setLeaving((l) => [...l, ...gone]);
    timers.current.push(window.setTimeout(() => setLeaving((l) => l.filter((t) => !gone.includes(t))), TOAST_LEAVE_MS));
  }, [toasts]);
  useEffect(() => {
    const all = timers.current;
    return () => all.forEach((t) => window.clearTimeout(t));
  }, []);
  // Each leaving toast keeps its place in the stack while it fades.
  const all = [...toasts, ...leaving].sort((a, b) => a.id - b.id);
  return (
    <div className="toasts" role="status" aria-live="polite">
      {all.map((t) => {
        const out = leaving.includes(t);
        return (
          <div key={t.id} className={`toast toast-${t.tone} ${out ? "toast-leave" : ""}`} aria-hidden={out || undefined}>
            {t.tone === "ok" ? <Icon name="checkCircle" size={14} /> : t.tone === "err" ? <Icon name="alert" size={14} /> : null}
            <span>{t.text}</span>
            {t.action ? <button className="toast-action" tabIndex={out ? -1 : undefined} onClick={() => { t.action?.run(); dismissToast(t.id); }}>{t.action.label}</button> : null}
            {t.ms === null ? <button className="toast-close" tabIndex={out ? -1 : undefined} aria-label="Dismiss" onClick={() => dismissToast(t.id)}><Icon name="close" size={13} /></button> : null}
          </div>
        );
      })}
    </div>
  );
}

// ---------------------------------------------------------------- Tooltips

/** How long the pointer rests on something before its tooltip shows (moving between tipped things shows at once). */
const TIP_DELAY_MS = 250;

/** The one tooltip layer: any element with `data-tip` gets its text in a tooltip to its right (or its left, if the
 *  window has no room), drawn above everything so no scrolling sidebar or panel clips it. It shows on hover after a
 *  short rest, at once on keyboard focus, and follows the element's `data-tip` while shown. Mounted once, in App. */
export function TipLayer() {
  // `key`: the element's data-tip-key, the key that does the same, drawn as a keycap after the text.
  const [tip, setTip] = useState<{ el: HTMLElement; text: string; key?: string } | null>(null);
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null);
  const box = useRef<HTMLDivElement>(null);

  useEffect(() => {
    let timer = 0;
    let current: HTMLElement | null = null;
    const show = (el: HTMLElement, now: boolean) => {
      window.clearTimeout(timer);
      current = el;
      // A `.tip-words` inside says the same as the tip: the tip shows only while a narrow layout hides those words.
      const said = (el.querySelector<HTMLElement>(":scope > .tip-words")?.offsetWidth ?? 0) > 0;
      const go = () => { if (current === el && el.dataset.tip && !said) setTip({ el, text: el.dataset.tip, key: el.dataset.tipKey }); };
      if (now) go(); else timer = window.setTimeout(go, TIP_DELAY_MS);
    };
    const hide = () => { window.clearTimeout(timer); current = null; setTip(null); };
    const tipped = (t: EventTarget | null) => (t instanceof Element ? t.closest<HTMLElement>("[data-tip]") : null);
    const over = (e: PointerEvent) => {
      const el = tipped(e.target);
      if (el === current) return;
      if (!el) return hide();
      show(el, current !== null);
    };
    const focus = (e: FocusEvent) => {
      const el = tipped(e.target);
      if (el && el.matches(":focus-visible")) show(el, true);
    };
    const key = (e: KeyboardEvent) => { if (e.key === "Escape") hide(); };
    document.addEventListener("pointerover", over, true);
    document.addEventListener("focusin", focus, true);
    document.addEventListener("focusout", hide, true);
    document.addEventListener("keydown", key, true);
    window.addEventListener("scroll", hide, true);
    window.addEventListener("blur", hide);
    return () => {
      window.clearTimeout(timer);
      document.removeEventListener("pointerover", over, true);
      document.removeEventListener("focusin", focus, true);
      document.removeEventListener("focusout", hide, true);
      document.removeEventListener("keydown", key, true);
      window.removeEventListener("scroll", hide, true);
      window.removeEventListener("blur", hide);
    };
  }, []);

  // While shown, follow the element: its text changing (an armed button), or it leaving the page.
  useEffect(() => {
    if (!tip) return;
    const follow = () => {
      const text = tip.el.dataset.tip;
      const key = tip.el.dataset.tipKey;
      if (!text || !tip.el.isConnected) setTip(null);
      else if (text !== tip.text || key !== tip.key) setTip({ el: tip.el, text, key });
    };
    const watch = new MutationObserver(follow);
    watch.observe(tip.el, { attributes: true, attributeFilter: ["data-tip", "data-tip-key"] });
    follow(); // it may have changed between showing and watching (a click that arms a button)
    return () => watch.disconnect();
  }, [tip]);

  useLayoutEffect(() => {
    if (!tip || !box.current) { setPos(null); return; }
    const r = tip.el.getBoundingClientRect();
    const w = box.current.offsetWidth;
    const right = r.right + 10;
    setPos({ left: right + w <= window.innerWidth - 8 ? right : Math.max(8, r.left - 10 - w), top: r.top + r.height / 2 });
  }, [tip]);

  if (!tip) return null;
  return createPortal(
    <div ref={box} className={`tip ${pos ? "tip-on" : ""}`} role="tooltip" style={pos ? { left: pos.left, top: pos.top } : { left: -9999, top: 0 }}>
      {tip.text}{tip.key ? <span className="kc">{tip.key}</span> : null}
    </div>,
    document.body,
  );
}

// ---------------------------------------------------------------- Switch

export function Switch({ checked, onChange, label, disabled }: { checked: boolean; onChange: (v: boolean) => void; label: string; disabled?: boolean }) {
  return (
    <label className="switch">
      <input type="checkbox" role="switch" checked={checked} disabled={disabled} aria-label={label} onChange={(e) => onChange(e.target.checked)} />
      <span />
    </label>
  );
}

// ---------------------------------------------------------------- Tabs

export function Tabs<T extends string>({ tabs, value, onChange, className }: {
  tabs: { id: T; label: string; badge?: number | string | null }[]; value: T; onChange: (t: T) => void; className?: string;
}) {
  const refs = useRef(new Map<T, HTMLButtonElement>());
  const [bar, setBar] = useState<{ left: number; width: number } | null>(null);
  useLayoutEffect(() => {
    const el = refs.current.get(value);
    if (el) setBar({ left: el.offsetLeft, width: el.offsetWidth });
  }, [value, tabs.length]);
  return (
    <div className={`tabs ${className ?? ""}`} role="tablist">
      {tabs.map((t, i) => (
        <button
          key={t.id} role="tab" aria-selected={t.id === value} tabIndex={t.id === value ? 0 : -1}
          ref={(el) => { if (el) refs.current.set(t.id, el); }}
          className="tab" onClick={() => onChange(t.id)}
          onKeyDown={(e) => {
            if (e.key !== "ArrowRight" && e.key !== "ArrowLeft") return;
            const next = tabs[(i + (e.key === "ArrowRight" ? 1 : tabs.length - 1)) % tabs.length];
            onChange(next.id);
            refs.current.get(next.id)?.focus();
          }}
        >
          {t.label}
          {t.badge ? <span className="tab-badge">{t.badge}</span> : null}
        </button>
      ))}
      {bar ? <span className="tabs-bar" style={{ transform: `translateX(${bar.left}px) scaleX(${bar.width})` }} /> : null}
    </div>
  );
}

// ---------------------------------------------------------------- page layout

/** A page's one header row: its title, and the actions that belong to this page only. */
export function PageHeader({ title, children }: { title: string; children?: ReactNode }) {
  return (
    <header className="pagehead">
      <h1 className="pagehead-title">{title}</h1>
      {children ? <div className="pagehead-actions">{children}</div> : null}
    </header>
  );
}

/** The one content column every page but the Org chart uses: a header that stays put over a column that scrolls. */
export function PageColumn({ title, children, className, actions }: { title: string; children: ReactNode; className?: string; actions?: ReactNode }) {
  // Content scrolling under the header fades at that edge (and at the foot while more is below), never sliced.
  const scroll = useScrollFade<HTMLDivElement>();
  return (
    <div className={`pagecol ${className ?? ""}`}>
      <PageHeader title={title}>{actions}</PageHeader>
      <div ref={scroll} className="pagecol-scroll scroll-fade">
        <div className="pagecol-inner">{children}</div>
      </div>
    </div>
  );
}

/** A plain titled section inside a page or panel: a quiet heading, no card. */
export function Section({ title, count, action, children, id, className }: {
  title: ReactNode; count?: number | null; action?: ReactNode; children: ReactNode; id?: string; className?: string;
}) {
  return (
    <section className={`psec ${className ?? ""}`} id={id}>
      <header className="psec-head">
        <h2 className="psec-title">{title}{count ? <span className="psec-count num"><Roll value={count} /></span> : null}</h2>
        {action ? <div className="psec-action">{action}</div> : null}
      </header>
      {children}
    </section>
  );
}

export type MenuItem = { label: string; icon?: IconName; onSelect: () => void; disabled?: boolean; danger?: boolean };

/** A "More" button whose menu holds the actions that do not need to be on screen all the time. */
export function MenuButton({ items, label }: { items: MenuItem[]; label: string }) {
  const [open, setOpen] = useState(false);
  const anchor = useRef<HTMLButtonElement>(null);
  const list = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (open) requestAnimationFrame(() => list.current?.querySelector<HTMLButtonElement>("button:not(:disabled)")?.focus());
  }, [open]);
  if (!items.length) return null;
  return (
    <>
      <button ref={anchor} type="button" className="btn btn-ghost btn-icon btn-sm" aria-label={label} title={label} aria-haspopup="menu" aria-expanded={open} onClick={() => setOpen((o) => !o)}>
        <Icon name="more" size={14} />
      </button>
      <Popover anchor={anchor} open={open} onClose={() => setOpen(false)} width={236}>
        <div
          ref={list} className="menu" role="menu" aria-label={label} data-esc-stop
          onKeyDown={(e) => {
            if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
            e.preventDefault();
            const btns = [...(list.current?.querySelectorAll<HTMLButtonElement>("button:not(:disabled)") ?? [])];
            const i = btns.indexOf(document.activeElement as HTMLButtonElement);
            btns[(i + (e.key === "ArrowDown" ? 1 : btns.length - 1)) % btns.length]?.focus();
          }}
        >
          {items.map((it) => (
            <button key={it.label} type="button" role="menuitem" className={`menu-item ${it.danger ? "menu-item-danger" : ""}`} disabled={it.disabled}
              onClick={() => { setOpen(false); it.onSelect(); }}>
              {it.icon ? <Icon name={it.icon} size={13} /> : null}{it.label}
            </button>
          ))}
        </div>
      </Popover>
    </>
  );
}

// ---------------------------------------------------------------- misc

/** Something failed. Inline, it is one line. With a `title` it is the full pattern: what happened (title), what happens
 *  next (text), and one way to act. */
export function ErrorNote({ text, title, onRetry }: { text: string; title?: string; onRetry?: () => void }) {
  const retry = onRetry ? <button className="btn btn-sm" onClick={onRetry}><Icon name="refresh" size={12} /> Try again</button> : null;
  if (title) {
    return (
      <div className="error-note error-note-block" role="alert">
        <Icon name="alert" size={15} />
        <div className="grow">
          <div className="error-note-title">{title}</div>
          <div className="error-note-text">{text}</div>
        </div>
        {retry}
      </div>
    );
  }
  return (
    <div className="error-note" role="alert">
      <Icon name="alert" size={14} />
      <span className="grow">{text}</span>
      {retry}
    </div>
  );
}

/** A block at the content's own geometry while it loads. It holds still. `r` defaults to the square corner; pass
 *  `r={h / 2}` for an avatar. */
export function Skeleton({ h = 14, w = "100%", r = 3 }: { h?: number; w?: number | string; r?: number }) {
  return <span className="skeleton" style={{ height: h, width: w, borderRadius: r }} />;
}

/** A state in a pill: a still dot, then the word. `wait` is work waiting to start ("Up next"): a hollow ring and a quiet
 *  word, never ultramarine. */
export function StatePill({ tone, children }: { tone: "accent" | "wait" | "ok" | "warn" | "err" | "neutral"; children: ReactNode }) {
  return (
    <span className={`pill ${tone === "neutral" ? "" : tone === "accent" ? "pill-live" : `pill-${tone}`} state-pill`}>
      <span className="state-dot" />
      {children}
    </span>
  );
}

/** What a value reads as a number: a number, or a string that is one with a sign or unit around it ("$2.94", "38%",
 *  "1,180"); NaN for anything else ("Working", "3m 04s"). */
const NUMBERISH = /^[^\d\s-]*-?\d[\d.,]*[^\d\s]*$/;
const numberOf = (v: string | number) => (typeof v === "number" ? v : NUMBERISH.test(v) ? parseFloat(v.replace(/[^\d.-]/g, "")) : NaN);

/** A value that rolls to its new value when it changes, in its own clipped slot: the old one leaves as the new one
 *  arrives, upward when the value went up and downward when it went down (numbers and amounts alike). A number or an
 *  amount rolls only the digits that changed, last digit first; unchanged digits hold still. Words roll whole. Under
 *  reduced motion the two crossfade. `render` draws each value (an icon with its word: they roll as one). */
export function Roll({ value, className, render }: { value: string | number; className?: string; render?: (v: string | number) => ReactNode }) {
  const draw = render ?? ((v: string | number) => v);
  const [roll, setRoll] = useState<{ value: string | number; from: string | number | null; up: boolean }>({ value, from: null, up: true });
  if (roll.value !== value) {
    const [a, b] = [numberOf(roll.value), numberOf(value)];
    setRoll({ value, from: roll.value, up: Number.isNaN(a) || Number.isNaN(b) ? true : b > a });
  }
  const slot = useRef<HTMLSpanElement>(null);
  useLayoutEffect(() => {
    const el = slot.current;
    if (roll.from === null || !el) return;
    // Changing slots, right to left: the last digit leads and each one to its left follows a beat later.
    const slots = [...el.querySelectorAll<HTMLElement>("[data-roll]")].reverse();
    const anims = slots.flatMap((s, i) => rollAnimations(s.children[0] as HTMLElement, s.children[1] as HTMLElement, roll.up, i * ms("--stagger")));
    let live = true;
    void Promise.all(anims.map((a) => a.finished)).then(() => { if (live) setRoll((r) => ({ ...r, from: null })); }, () => {});
    return () => { live = false; anims.forEach((a) => a.cancel()); };
  }, [roll.from, roll.up]);
  if (roll.from === null) return <span ref={slot} className={`roll ${className ?? ""}`}><span>{draw(value)}</span></span>;
  const [from, to] = [String(roll.from), String(value)];
  if (render || !NUMBERISH.test(from) || !NUMBERISH.test(to)) {
    return (
      <span ref={slot} className={`roll ${className ?? ""}`}>
        <span className="roll" data-roll><span aria-hidden>{draw(roll.from)}</span><span>{draw(value)}</span></span>
      </span>
    );
  }
  // Digit by digit, lined up from the right so units, tens and cents meet their own.
  const n = Math.max(from.length, to.length);
  const at = (t: string, i: number) => t[i - (n - t.length)] ?? "";
  return (
    <span ref={slot} className={`roll-digits ${className ?? ""}`}>
      {Array.from({ length: n }, (_, i) => {
        const [a, b] = [at(from, i), at(to, i)];
        return a === b ? <span key={i}>{b}</span> : <span key={i} className="roll" data-roll><span aria-hidden>{a}</span><span>{b}</span></span>;
      })}
    </span>
  );
}
