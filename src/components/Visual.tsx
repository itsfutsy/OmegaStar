import { useEffect, useRef, useState } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { useResolvedDark } from "../lib/theme";
import { openWeb, webUrl } from "./conversation/CardLink";
import { Icon } from "./Icon";
import { Overlay } from "./ui";
import "./Visual.css";

/** Tallest a page grows in a message. Past it, it scrolls in its frame. */
const MAX_HEIGHT = 2000;

/** The least time between two links a page gets opened, in ms, so a page that keeps asking cannot flood the browser. */
const OPEN_GAP = 1000;

/** Where a page is served: a scheme of its own, so its frame shares nothing with the app (`visual_page` in Rust). */
const pageUrl = (runId: string, id: string, dark: boolean) =>
  `${convertFileSrc(`${runId}/${id}`, "omega-visual")}?theme=${dark ? "dark" : "light"}`;

/**
 * A page an employee showed (`show`), as tall as it is, with Expand for a full-window view. Its frame runs the
 * page's own scripts and nothing else: no network, no access to the app, no origin of its own. A click on a web
 * link in it opens that page in your browser.
 */
export function Visual({ runId, id, title, from }: { runId: string; id: string; title: string; from?: string }) {
  const src = pageUrl(runId, id, useResolvedDark());
  const frame = useRef<HTMLIFrameElement>(null);
  const sheet = useRef<HTMLIFrameElement>(null);
  const opened = useRef(0);
  const [height, setHeight] = useState<number | null>(null);
  const [open, setOpen] = useState(false);
  useEffect(() => {
    const onMessage = (e: MessageEvent) => {
      const data = e.data as { omegaHeight?: unknown; omegaOpen?: unknown } | null;
      const sender = [frame.current, sheet.current].find((f) => e.source !== null && e.source === f?.contentWindow);
      if (!sender) return;
      if (sender === frame.current && typeof data?.omegaHeight === "number") setHeight(Math.min(MAX_HEIGHT, Math.max(24, data.omegaHeight)));
      // A page's script can send a link with no click behind it, so one opens only while the pointer is over the page
      // and you have just clicked or pressed a key, and no sooner than a second after the last one.
      const clicked = sender.matches(":hover") && navigator.userActivation?.isActive && performance.now() - opened.current >= OPEN_GAP;
      const link = clicked && typeof data?.omegaOpen === "string" ? webUrl(data.omegaOpen) : null;
      if (!link) return;
      opened.current = performance.now();
      openWeb(link);
    };
    window.addEventListener("message", onMessage);
    return () => window.removeEventListener("message", onMessage);
  }, []);
  const heading = <span className="visual-title"><span className="trunc">{title}</span>{from ? <span className="visual-from">from {from}</span> : null}</span>;
  return (
    <figure className="visual">
      <figcaption className="visual-head">
        {heading}
        <button className="btn btn-ghost btn-icon btn-sm" aria-label={`Expand ${title}`} data-tip="Expand" onClick={() => setOpen(true)}><Icon name="expand" size={14} /></button>
      </figcaption>
      <div className="visual-body">
        <iframe ref={frame} className={`visual-frame ${height === null ? "visual-wait" : ""}`} src={src} sandbox="allow-scripts" title={title} style={{ height: height ?? 160 }} />
      </div>
      {open ? (
        <Overlay label={title} onClose={() => setOpen(false)}>
          <div className="visual-sheet">
            <div className="visual-head">{heading}</div>
            <div className="visual-body"><iframe ref={sheet} className="visual-frame" src={src} sandbox="allow-scripts" title={title} /></div>
          </div>
        </Overlay>
      ) : null}
    </figure>
  );
}
