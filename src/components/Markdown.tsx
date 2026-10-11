import { useLayoutEffect, useMemo, useRef } from "react";
import { Marked } from "marked";
import DOMPurify from "dompurify";
import { call, errorMessage } from "../api/client";
import { toast } from "../lib/toasts";

const IMAGE_PATH = /^\/\S*\.(png|jpe?g|gif|webp)$/i;
const esc = (s: string) => s.replace(/[&<>"']/g, (c) => `&#${c.charCodeAt(0)};`);
const imageCode = (path: string) => `<code data-image-path="${esc(path)}">${esc(path)}</code>`;

const md = new Marked({ gfm: true, breaks: false, async: false });
md.use({
  renderer: {
    codespan: ({ text }) => IMAGE_PATH.test(text) && imageCode(text),
    image: ({ href }) => IMAGE_PATH.test(href) && imageCode(href),
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
    if (e.target instanceof HTMLImageElement && e.target.classList.contains("md-img"))
      call("open_image", { path: e.target.alt }).catch((err) => toast(errorMessage(err), "err"));
  };
  return <div ref={ref} className={`md ${className ?? ""}`} onClick={open} dangerouslySetInnerHTML={{ __html: html }} />;
}
