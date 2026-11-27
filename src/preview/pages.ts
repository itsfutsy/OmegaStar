// What the preview shows where the desktop app reads from disk: the page an employee showed (`show`), and the
// images cards and messages point at. Plus the pieces of Tauri the UI calls directly, outside the transport.
import tokens from "../design/tokens.css?raw";
import interOpsz from "@fontsource-variable/inter/files/inter-latin-opsz-normal.woff2?inline";

/** The pages employees showed, by visual id. Served the way the app serves them: OmegaStar's tokens first. */
const PAGES: Record<string, string> = {
  timeline: `
<style>
  /* The first stop on the frame title's text edge (14px), 22px of air above the dots and 17px under the labels. */
  .tl { display: grid; grid-template-columns: repeat(4, 1fr); padding: 16px 14px 17px; }
  .step { position: relative; padding: 22px 12px 0 0; }
  .step::before { content: ""; position: absolute; top: 6px; left: 0; right: 0; height: 2px; background: var(--line-strong); }
  .step:last-child::before { right: auto; width: 14px; }
  .dot { position: absolute; top: 1px; left: 0; width: 8px; height: 8px; border-radius: 50%; background: var(--canvas); border: 2px solid var(--ink-2); }
  .step.key .dot { background: var(--live); border-color: var(--live); }
  .when { font-size: 12px; color: var(--muted); font-variant-numeric: tabular-nums; }
  .what { font-weight: 560; margin-top: 2px; }
  .who { font-size: 12px; color: var(--muted); margin-top: 2px; }
  /* Narrow (a person's page): two by two, so no label wraps. */
  @media (max-width: 520px) {
    .tl { grid-template-columns: repeat(2, 1fr); row-gap: 18px; }
    .step:nth-child(2n)::before { right: auto; width: 14px; }
  }
</style>
<div class="tl">
  <div class="step"><span class="dot"></span><div class="when">24 Jan</div><div class="what">Board update</div><div class="who">Juniper, to Harriet</div></div>
  <div class="step"><span class="dot"></span><div class="when">27 Jan</div><div class="what">Help centre</div><div class="who">Mae, 3 articles</div></div>
  <div class="step key"><span class="dot"></span><div class="when">30 Jan</div><div class="what">Email to studios</div><div class="who">212 studios</div></div>
  <div class="step"><span class="dot"></span><div class="when">1 Mar</div><div class="what">$34 a month</div><div class="who">Annual plans at renewal</div></div>
</div>`,
};

const HEIGHT = "new ResizeObserver(() => parent.postMessage({ omegaHeight: Math.ceil(document.documentElement.getBoundingClientRect().height) }, '*')).observe(document.documentElement);";
const LINKS = "addEventListener('click', (e) => { const a = e.target.closest?.('a[href]'); const url = a && String(a.href.baseVal ?? a.href); if (url && /^https?:/.test(url) && !url.startsWith(location.origin + '/') && !e.defaultPrevented) { e.preventDefault(); parent.postMessage({ omegaOpen: url }, '*'); } });";
// The UI adds `?theme=dark` to the address; in a data URL that lands at the end of the page, so the page ends in an
// open comment that swallows it, and reads the theme from its own address.
const THEME = "if (/theme=dark$/.test(location.href)) document.documentElement.dataset.theme = 'dark';";
// As src-tauri/src/office/visuals.rs serves a page: Inter with its optical-size axis embedded, the app's type features.
const BASE = `@font-face { font-family: "Inter Variable"; font-style: normal; font-weight: 100 900; font-display: block; src: url(${interOpsz}) format("woff2"); }
html { color: var(--ink); background: transparent; font-family: var(--font-ui); font-size: var(--t-base); line-height: 1.5; font-feature-settings: var(--font-features); -webkit-font-smoothing: antialiased; } body { margin: 0; }`;

function pageUrl(id: string): string {
  const page = PAGES[id] ?? `<p style="padding:16px;color:var(--muted)">This page is no longer there.</p>`;
  const html = `<!doctype html><html data-theme="light"><head><meta charset="utf-8"><style>${tokens}\n${BASE}</style><script>${THEME}${HEIGHT}${LINKS}</script></head><body>${page}</body></html><!--`;
  return `data:text/html;charset=utf-8,${encodeURIComponent(html)}`;
}

/** The UI's calls into Tauri outside the transport: a page's address (`convertFileSrc`), and the Dock badge and opening
 *  a link, which do nothing here. `inTauri` was decided before this loads, so the transport stays the preview's. */
export function installTauriShim() {
  const w = window as unknown as Record<string, unknown>;
  if (w.__TAURI_INTERNALS__) return;
  w.__TAURI_INTERNALS__ = {
    convertFileSrc: (path: string, protocol: string) => (protocol === "omega-visual" ? pageUrl(path.split("/").pop() ?? "") : path),
    metadata: { currentWindow: { label: "main" } },
    invoke: () => Promise.resolve(null),
  };
}

/** An invented picture for a local image path: soft shapes in a hue the path picks, like a product shot. */
export function imageUrl(path: string): string {
  let h = 0;
  for (const c of path) h = (h * 31 + c.charCodeAt(0)) % 360;
  const bg = `hsl(${h} 30% 92%)`, ink = `hsl(${h} 25% 30%)`, soft = `hsl(${h} 35% 80%)`, accent = `hsl(${(h + 150) % 360} 45% 55%)`;
  const rows = [0, 1, 2, 3].map((i) => `<rect x="236" y="${118 + i * 46}" width="${300 - (i % 2) * 70}" height="30" rx="8" fill="${i === 1 ? accent : "#fff"}"/>`).join("");
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="640" height="400" viewBox="0 0 640 400">
<rect width="640" height="400" fill="${bg}"/>
<rect x="72" y="64" width="496" height="296" rx="18" fill="#fff" opacity="0.65"/>
<rect x="104" y="96" width="104" height="232" rx="12" fill="${soft}"/>
<circle cx="156" cy="146" r="26" fill="${ink}" opacity="0.85"/>
<rect x="128" y="196" width="56" height="10" rx="5" fill="${ink}" opacity="0.5"/>
<rect x="236" y="86" width="140" height="14" rx="7" fill="${ink}" opacity="0.7"/>
${rows}
</svg>`;
  return `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`;
}
