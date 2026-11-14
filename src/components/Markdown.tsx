import { useLayoutEffect, useMemo, useRef, useState } from "react";
import { Marked, type Tokens } from "marked";
import DOMPurify from "dompurify";
import { call, errorMessage } from "../api/client";
import { toast } from "../lib/toasts";
import { Lightbox, loadImage, type Shot } from "./Lightbox";

const EXT = "png|jpe?g|gif|webp";
/** Where a full local path starts: `/`, `~/` or a drive (`C:\`). */
const ROOT = String.raw`(?:/|~/|[A-Za-z]:[\\/])`;
/** A local image path, spaces and all ("/Users/me/Library/Application Support/shot.png"). */
const IMAGE_PATH = new RegExp(String.raw`^${ROOT}[^\n]*\.(?:${EXT})$`, "i");
/** `![caption](/full/path.png)`, which marked leaves as text when the path has a space in it. */
const IMAGE_MD = new RegExp(String.raw`^!\[([^\]\n]*)\]\(\s*<?(${ROOT}[^\n]*?\.(?:${EXT}))>?\s*\)`, "i");
// A path written out at the end of its line ("Before: /Users/me/shot.png"). It starts after a space and never
// crosses a space followed by another root, so "see /etc/hosts and /tmp/shot.png" shows /tmp/shot.png.
const BARE = String.raw`${ROOT}(?:(?!\s${ROOT})[^\n])*?\.(?:${EXT})(?=[.,;)]?[ \t]*(?:\n|$))`;
const BARE_IMAGE = new RegExp(`^${BARE}`, "i");
const BARE_START = new RegExp(String.raw`(?:^|\s)(${BARE})`, "im");
const esc = (s: string) => s.replace(/[&<>"']/g, (c) => `&#${c.charCodeAt(0)};`);
const imageCode = (path: string, alt = "") =>
  `<code data-image-path="${esc(path)}"${alt.trim() ? ` data-alt="${esc(alt.trim())}"` : ""}>${esc(path)}</code>`;
// A local path an agent mentioned. It opens through Rust on click, so it has no href:
// DOMPurify drops file: hrefs and a bare /Users/... href would resolve inside the webview.
const OPEN_PATH = /^(\/|~\/)\S+$/;
const pathAttrs = (path: string) => `class="md-path" data-open-path="${esc(path)}" title="Open"`;
// A file URL is percent-encoded (a space is %20), and so is a path in a markdown link or image, by its writer or by
// marked. A path that does not decode is taken as written.
const decoded = (s: string) => {
  try {
    return decodeURIComponent(s);
  } catch {
    return s;
  }
};
const linkPath = (href: string) => (href.startsWith("file://") ? decoded(href.slice(7)) : /^(\/|~\/)/.test(href) ? decoded(href) : null);

const md = new Marked({ gfm: true, breaks: false, async: false });
md.use({
  extensions: [
    {
      name: "imageMd",
      level: "inline",
      start: (src) => src.indexOf("!["),
      tokenizer(src) {
        const m = IMAGE_MD.exec(src);
        return m ? { type: "imageMd", raw: m[0], alt: m[1], path: decoded(m[2]) } : undefined;
      },
      renderer: (t: Tokens.Generic) => imageCode(t.path, t.alt),
    },
    {
      name: "imagePath",
      level: "inline",
      start(src) {
        const m = BARE_START.exec(src);
        return m ? m.index + m[0].length - m[1].length : undefined;
      },
      tokenizer(src) {
        const m = BARE_IMAGE.exec(src);
        return m ? { type: "imagePath", raw: m[0], path: m[0] } : undefined;
      },
      renderer: (t: Tokens.Generic) => imageCode(t.path),
    },
  ],
  renderer: {
    codespan: ({ text }) =>
      IMAGE_PATH.test(text) ? imageCode(text) : OPEN_PATH.test(text) && `<code ${pathAttrs(text)}>${esc(text)}</code>`,
    image: ({ href, text }) => IMAGE_PATH.test(href) && imageCode(decoded(href), text),
    link({ href, tokens }) {
      const path = linkPath(href);
      return path !== null && `<a ${pathAttrs(path)}>${this.parser.parseInline(tokens)}</a>`;
    },
  },
});

DOMPurify.addHook("afterSanitizeAttributes", (node) => {
  if (node.tagName === "A") {
    node.setAttribute("target", "_blank");
    node.setAttribute("rel", "noreferrer noopener");
  }
});

function renderMarkdown(src: string, breaks: boolean): string {
  const html = md.parse(src, { breaks }) as string;
  const doc = DOMPurify.sanitize(html, { USE_PROFILES: { html: true }, RETURN_DOM_FRAGMENT: true });
  galleries(doc);
  letters(doc);
  wholeIds(doc);
  scrollTables(doc);
  const out = document.createElement("div");
  out.append(doc);
  return out.innerHTML;
}

/** A cell this short (a date, a state, a number) stays on one line. */
const SHORT_CELL = 16;

/** Each table sits in a strip of its own that scrolls sideways, so a wide one keeps its columns rather than squeezing
 *  them to a letter a line, and its short cells keep to one line. */
function scrollTables(root: DocumentFragment) {
  for (const cell of root.querySelectorAll("th, td")) if ((cell.textContent ?? "").trim().length <= SHORT_CELL) cell.classList.add("md-short");
  for (const table of root.querySelectorAll("table")) {
    const strip = document.createElement("div");
    strip.className = "md-table";
    table.before(strip);
    strip.append(table);
  }
}

export const ID_PATTERN = /\b([A-Z][A-Z0-9]*-\d+)\b/;

/** Ids like INV-2291 or PR-88 stay whole on one line: never broken at their hyphen. */
function wholeIds(root: DocumentFragment) {
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  const hits: Text[] = [];
  for (let n = walker.nextNode(); n; n = walker.nextNode()) if (ID_PATTERN.test(n.textContent ?? "") && !n.parentElement?.closest("code, pre")) hits.push(n as Text);
  for (const t of hits) {
    const parts = (t.textContent ?? "").split(ID_PATTERN);
    t.replaceWith(...parts.map((p, i) => {
      if (i % 2 === 0) return document.createTextNode(p);
      const s = document.createElement("span");
      s.style.whiteSpace = "nowrap";
      s.textContent = p;
      return s;
    }));
  }
}

/** A quoted draft that opens "**Subject:** …" reads as a letter: the subject becomes its own line ("Subject" then the
 *  words), so a conversation can set it apart from the body. */
function letters(root: DocumentFragment) {
  for (const q of root.querySelectorAll("blockquote")) {
    const p = q.firstElementChild;
    const lead = p?.tagName === "P" ? p.firstChild : null;
    if (!p || !(lead instanceof HTMLElement) || lead.tagName !== "STRONG" || !/^subject:?$/i.test(lead.textContent?.trim() ?? "")) continue;
    q.classList.add("md-letter");
    p.classList.add("md-subj");
    // The subject runs to the end of its line; anything after it on the same paragraph becomes the letter's first one.
    const rest = [...p.childNodes].slice(1);
    let cut = rest.findIndex((n) => n.nodeName === "BR" || (n.nodeType === Node.TEXT_NODE && (n.textContent ?? "").includes("\n")));
    const after: Node[] = [];
    if (cut >= 0) {
      const n = rest[cut];
      if (n.nodeType === Node.TEXT_NODE) {
        const [head, ...tail] = (n.textContent ?? "").split("\n");
        rest[cut] = document.createTextNode(head);
        after.push(document.createTextNode(tail.join("\n")));
        cut++;
      }
      after.push(...rest.slice(cut + (n.nodeName === "BR" ? 1 : 0)));
      rest.length = cut;
    }
    const label = document.createElement("span");
    label.textContent = "Subject";
    const words = document.createElement("b");
    words.textContent = rest.map((n) => n.textContent).join("").trim();
    p.replaceChildren(label, words);
    if (after.some((n) => (n.textContent ?? "").trim())) {
      const next = document.createElement("p");
      next.append(...after);
      p.after(next);
    }
  }
}

/** Longest caption shown under an image. Longer, it stays a paragraph above it. */
const CAPTION_MAX = 160;
const isImage = (n: Node): n is HTMLElement => n instanceof HTMLElement && n.matches("code[data-image-path]");
const blank = (n: Node) => n.nodeName === "BR" || (n.nodeType === Node.TEXT_NODE && /^[\s.,;]*$/.test(n.textContent ?? ""));

/** Paragraphs of images, each after its caption ("Before: /a.png", or `![Before](/a.png)`), become figures, and
 *  figures next to each other a gallery: one image full size, more in a grid of two or three across. */
function galleries(root: DocumentFragment) {
  for (const p of [...root.children]) {
    const figs = p.tagName === "P" ? figures(p) : null;
    if (figs) p.replaceWith(...figs);
  }
  let row: Element[] = [];
  const flush = () => {
    if (!row.length) return;
    const g = document.createElement("div");
    g.className = `md-gallery md-gallery-${row.length === 1 ? 1 : row.length === 2 || row.length === 4 ? 2 : 3}`;
    row[0].before(g);
    g.append(...row);
    row = [];
  };
  for (const n of [...root.childNodes]) {
    if (n instanceof HTMLElement && n.classList.contains("md-fig")) row.push(n);
    else if (!blank(n)) flush();
  }
  flush();
}

/** A paragraph's images as figures with their captions, or null when it is prose with an image in it: every image
 *  must end its line. */
function figures(p: Element): Element[] | null {
  const all = [...p.childNodes];
  const last = all.map(isImage).lastIndexOf(true);
  const endsLine = (n: Node) => {
    const next = n.nextSibling;
    return !next || next.nodeName === "BR" || (next.nodeType === Node.TEXT_NODE && /^[.,;)]?[ \t]*(\n|$)/.test(next.textContent!));
  };
  if (last < 0 || !all.filter(isImage).every(endsLine) || !all.slice(last + 1).every(blank)) return null;
  const out: Element[] = [];
  let caption: Node[] = [];
  for (const n of all.slice(0, last + 1)) {
    if (!isImage(n)) {
      caption.push(n);
      continue;
    }
    // The caption is the image's own line ("Before: /a.png"), else the line above it ("Before:" then the path).
    let [before, line] = lastLine(caption);
    if (!words(line)) [before, line] = lastLine(before);
    if (words(before)) out.push(para(before));
    if (words(line).length > CAPTION_MAX) out.push(para(line), figure(n, []));
    else out.push(figure(n, trim(line, true)));
    caption = [];
  }
  return out;
}

const words = (nodes: Node[]) => nodes.map((n) => n.textContent).join("").trim();
const para = (nodes: Node[]) => {
  const p = document.createElement("p");
  p.append(...trim(nodes));
  return p;
};

/** Inline nodes split at their last line break: what comes before it, and the last line. */
function lastLine(nodes: Node[]): [Node[], Node[]] {
  for (let i = nodes.length - 1; i >= 0; i--) {
    const n = nodes[i];
    if (n.nodeName === "BR") return [nodes.slice(0, i), nodes.slice(i + 1)];
    const at = n.nodeType === Node.TEXT_NODE ? n.textContent!.lastIndexOf("\n") : -1;
    if (at >= 0) {
      const t = n.textContent!;
      return [[...nodes.slice(0, i), document.createTextNode(t.slice(0, at))], [document.createTextNode(t.slice(at + 1)), ...nodes.slice(i + 1)]];
    }
  }
  return [[], nodes];
}

/** Caption nodes without the blank ones around them, and, with `colon`, the colon that led into the image. */
function trim(nodes: Node[], colon = false): Node[] {
  const out = [...nodes];
  while (out.length && blank(out[0])) out.shift();
  while (out.length && blank(out[out.length - 1])) out.pop();
  const [first, last] = [out[0], out[out.length - 1]];
  if (first?.nodeType === Node.TEXT_NODE) first.textContent = first.textContent!.replace(/^[.,;)]?\s*/, "");
  if (last?.nodeType === Node.TEXT_NODE) last.textContent = last.textContent!.replace(colon ? /\s*:?\s*$/ : /\s+$/, "");
  return out;
}

/** An image with its caption: the words before it, else its alt text. */
function figure(img: HTMLElement, caption: Node[]): HTMLElement {
  const fig = document.createElement("figure");
  fig.className = "md-fig";
  fig.append(img);
  const alt = img.dataset.alt;
  if (caption.some((n) => n.textContent?.trim()) || alt) {
    const cap = document.createElement("figcaption");
    if (caption.some((n) => n.textContent?.trim())) cap.append(...caption);
    else cap.textContent = alt!;
    fig.append(cap);
  }
  return fig;
}

/** `breaks`: a single line break stays one, as in a draft whose sign-off is on lines of its own. */
export function Markdown({ text, className, breaks = false }: { text: string; className?: string; breaks?: boolean }) {
  const html = useMemo(() => renderMarkdown(text, breaks), [text, breaks]);
  const ref = useRef<HTMLDivElement>(null);
  const [view, setView] = useState<{ shots: Shot[]; at: number } | null>(null);
  // A layout effect, so cached images swap in before paint and streamed re-renders do not flicker.
  useLayoutEffect(() => {
    for (const code of ref.current!.querySelectorAll<HTMLElement>("code[data-image-path]")) {
      const path = code.dataset.imagePath!;
      loadImage(path).then((src) => {
        if (!code.isConnected) return;
        const img = document.createElement("img");
        img.className = "md-img";
        img.src = src;
        img.dataset.path = img.title = path;
        img.alt = img.dataset.caption = code.closest("figure")?.querySelector("figcaption")?.textContent?.trim() ?? code.dataset.alt ?? "";
        code.replaceWith(img);
      }, () => code.isConnected && code.classList.add("md-img-missing"));
    }
  }, [html]);
  const open = (e: React.MouseEvent) => {
    const path = (e.target as Element).closest<HTMLElement>("[data-open-path]");
    if (path) {
      e.preventDefault();
      call("open_report_path", { path: path.dataset.openPath! }).catch((err) => toast(errorMessage(err), "err"));
    } else if (e.target instanceof HTMLImageElement && e.target.classList.contains("md-img")) {
      const all = [...ref.current!.querySelectorAll<HTMLImageElement>("img.md-img")];
      setView({ shots: all.map((img) => ({ path: img.dataset.path!, caption: img.dataset.caption ?? "" })), at: all.indexOf(e.target) });
    }
  };
  return (
    <>
      <div ref={ref} className={`md ${className ?? ""}`} onClick={open} dangerouslySetInnerHTML={{ __html: html }} />
      {view ? <Lightbox shots={view.shots} at={view.at} onClose={() => setView(null)} /> : null}
    </>
  );
}
