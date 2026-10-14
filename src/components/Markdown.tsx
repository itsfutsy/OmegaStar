import { useLayoutEffect, useMemo, useRef } from "react";
import { Marked } from "marked";
import DOMPurify from "dompurify";
import { call, errorMessage } from "../api/client";
import { toast } from "../lib/toasts";

const IMAGE_PATH = /^\/\S*\.(png|jpe?g|gif|webp)$/i;
const esc = (s: string) => s.replace(/[&<>"']/g, (c) => `&#${c.charCodeAt(0)};`);
const imageCode = (path: string) => `<code data-image-path="${esc(path)}">${esc(path)}</code>`;
// A local path an agent mentioned. It opens through Rust on click, so it has no href:
// DOMPurify drops file: hrefs and a bare /Users/... href would resolve inside the webview.
const OPEN_PATH = /^(\/|~\/)\S+$/;
const pathAttrs = (path: string) => `class="md-path" data-open-path="${esc(path)}" title="Open"`;
// A file URL is percent-encoded (a space is %20); a bare path is taken as written.
const decoded = (s: string) => {
  try {
    return decodeURIComponent(s);
  } catch {
    return s;
  }
};
const linkPath = (href: string) => (href.startsWith("file://") ? decoded(href.slice(7)) : /^(\/|~\/)/.test(href) ? href : null);

const md = new Marked({ gfm: true, breaks: false, async: false });
md.use({
  renderer: {
    codespan: ({ text }) =>
      IMAGE_PATH.test(text) ? imageCode(text) : OPEN_PATH.test(text) && `<code ${pathAttrs(text)}>${esc(text)}</code>`,
    image: ({ href }) => IMAGE_PATH.test(href) && imageCode(href),
    link({ href, tokens }) {
      const path = linkPath(href);
      return path !== null && `<a ${pathAttrs(path)}>${this.parser.parseInline(tokens)}</a>`;
    },
  },
});

const images = new Map<string, Promise<string>>();

function loadImage(path: string): Promise<string> {
  let p = images.get(path);
  if (!p) {
    p = call("read_image", { path });
    p.catch(() => images.delete(path));
    images.set(path, p);
    if (images.size > 20) images.delete(images.keys().next().value!);
  }
  return p;
}

DOMPurify.addHook("afterSanitizeAttributes", (node) => {
  if (node.tagName === "A") {
    node.setAttribute("target", "_blank");
    node.setAttribute("rel", "noreferrer noopener");
  }
});

function renderMarkdown(src: string): string {
  const html = md.parse(src) as string;
  return DOMPurify.sanitize(html, { USE_PROFILES: { html: true } });
}

/** Split YAML frontmatter from a SKILL.md body. */
export function splitFrontmatter(src: string): { meta: Record<string, string>; body: string } {
  const m = /^---\r?\n([\s\S]*?)\r?\n---\r?\n?/.exec(src);
  if (!m) return { meta: {}, body: src };
  const meta: Record<string, string> = {};
  for (const line of m[1].split(/\r?\n/)) {
    const i = line.indexOf(":");
    if (i > 0) meta[line.slice(0, i).trim()] = line.slice(i + 1).trim().replace(/^["']|["']$/g, "");
  }
  return { meta, body: src.slice(m[0].length) };
}

export function Markdown({ text, className }: { text: string; className?: string }) {
  const html = useMemo(() => renderMarkdown(text), [text]);
  const ref = useRef<HTMLDivElement>(null);
  // A layout effect, so cached images swap in before paint and streamed re-renders do not flicker.
  useLayoutEffect(() => {
    for (const code of ref.current!.querySelectorAll<HTMLElement>("code[data-image-path]")) {
      const path = code.dataset.imagePath!;
      loadImage(path).then((src) => {
        if (!code.isConnected) return;
        const img = document.createElement("img");
        img.className = "md-img";
        img.src = src;
        img.alt = img.title = path;
        code.replaceWith(img);
      }, () => {});
    }
  }, [html]);
  const open = (e: React.MouseEvent) => {
    const path = (e.target as Element).closest<HTMLElement>("[data-open-path]");
    if (path) {
      e.preventDefault();
      call("open_report_path", { path: path.dataset.openPath! }).catch((err) => toast(errorMessage(err), "err"));
    } else if (e.target instanceof HTMLImageElement && e.target.classList.contains("md-img"))
      call("open_image", { path: e.target.alt }).catch((err) => toast(errorMessage(err), "err"));
  };
  return <div ref={ref} className={`md ${className ?? ""}`} onClick={open} dangerouslySetInnerHTML={{ __html: html }} />;
}
