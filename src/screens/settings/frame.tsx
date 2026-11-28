import { useCallback, useEffect, useRef, useState, type CSSProperties, type ReactNode, type RefObject } from "react";
import { reducedMotion } from "../../lib/motion";
import { Section, Skeleton } from "../../components/ui";

const SECTIONS = ["Claude Code", "Running", "Notifications", "Replies from people", "Phone", "Appearance", "Data and backups", "CLI proxy", "About"] as const;
type SectionName = (typeof SECTIONS)[number];
const secId = (t: SectionName) => `set-${t.toLowerCase().replace(/[^a-z]+/g, "-")}`;

/** A section: its title (and a link beside it), an optional line on what it is for, its rows, then a note. */
export function Group({ title, sub, action, foot, children }: { title: SectionName; sub?: string; action?: ReactNode; foot?: ReactNode; children: ReactNode }) {
  return (
    <Section title={title} action={action} id={secId(title)} className="set-sec">
      {sub ? <p className="set-sub">{sub}</p> : null}
      <div className="set-list">{children}</div>
      {foot}
    </Section>
  );
}

/** The sections by name beside the page on wide windows; the one you are reading is marked, and a click goes there. */
export function SectionIndex({ host }: { host: RefObject<HTMLDivElement | null> }) {
  const [at, setAt] = useState(0);
  const pinned = useRef(false);
  const idle = useRef<number | undefined>(undefined);
  // A click marks its section at once and holds it while the page scrolls there.
  const settle = useCallback(() => {
    window.clearTimeout(idle.current);
    idle.current = window.setTimeout(() => { pinned.current = false; }, 160);
  }, []);
  useEffect(() => {
    const root = host.current;
    const scroller = root?.closest<HTMLElement>(".pagecol-scroll");
    if (!root || !scroller) return;
    const update = () => {
      if (pinned.current) return settle();
      // The section whose title has passed a line a quarter of the way down; at the end of the page the last one.
      const secs = [...root.querySelectorAll<HTMLElement>(".set-main > .set-sec")];
      const end = scroller.scrollTop > 0 && scroller.scrollTop + scroller.clientHeight >= scroller.scrollHeight - 2;
      const line = scroller.getBoundingClientRect().top + scroller.clientHeight * 0.25;
      let i = 0;
      secs.forEach((el, n) => { if (el.getBoundingClientRect().top <= line) i = n; });
      setAt(end ? secs.length - 1 : i);
    };
    update();
    scroller.addEventListener("scroll", update, { passive: true });
    return () => { scroller.removeEventListener("scroll", update); window.clearTimeout(idle.current); };
  }, [host, settle]);
  const go = (i: number) => {
    pinned.current = true;
    settle();
    setAt(i);
    document.getElementById(secId(SECTIONS[i]))?.scrollIntoView({ block: "start", behavior: reducedMotion() ? "auto" : "smooth" });
  };
  return (
    <nav className="set-index" aria-label="Settings sections">
      <span className="set-index-bar" style={{ "--at": at } as CSSProperties} aria-hidden />
      {SECTIONS.map((t, i) => (
        <button key={t} type="button" className="set-index-item" aria-current={i === at ? "location" : undefined} onClick={() => go(i)}>{t}</button>
      ))}
    </nav>
  );
}

/** What loading stands in for: the first four sections' own rows (a title, each line of description, and the
 *  control's blocks as [width, height, radius], with a second line of blocks for Business hours), so the page lands
 *  where the skeleton was and the skeleton runs off the bottom at every size. */
type Block = [number, number, number];
const LOADING: { head: number; link?: number; sub?: number[]; rows: { t: number; d: number[]; c: Block[]; c2?: Block[] }[] }[] = [
  { head: 104, link: 116, rows: [
    { t: 78, d: [214], c: [[76, 22, 11]] },
    { t: 98, d: [252], c: [[260, 32, 5], [60, 32, 5]] },
    { t: 62, d: [236], c: [[86, 22, 11]] },
    { t: 44, d: [196], c: [[46, 10, 3], [104, 32, 5]] },
  ] },
  { head: 72, rows: [
    { t: 292, d: [430, 104], c: [[28, 16, 8]] },
    { t: 142, d: [348], c: [[28, 16, 8]] },
    { t: 158, d: [436, 240], c: [[90, 32, 5]] },
  ] },
  { head: 104, sub: [300], rows: [
    { t: 150, d: [420, 96], c: [[124, 28, 5]] },
    { t: 88, d: [440, 76], c: [[140, 32, 5], [14, 10, 3], [140, 32, 5], [28, 16, 8]] },
  ] },
  { head: 160, sub: [440, 170], rows: [
    { t: 112, d: [420, 140], c: [[140, 32, 5], [14, 10, 3], [140, 32, 5]], c2: [[248, 24, 4]] },
    { t: 120, d: [400, 76], c: [[188, 28, 5]] },
  ] },
];
const INDEX_WIDTHS = [76, 50, 76, 112, 40, 72, 98, 56, 40];

/** Loading: the first four sections at their own geometry and the section list's place, held still. */
export function SettingsLoading() {
  return (
    <div className="set-page set-loading" aria-busy="true" aria-label="Loading settings">
      <div className="set-main">
        {LOADING.map((sec, s) => (
          <div key={s} className="set-sec">
            <div className="set-sk-head">
              <Skeleton h={13} w={sec.head} />
              {sec.link ? <Skeleton h={10} w={sec.link} /> : null}
            </div>
            {sec.sub ? <div className="set-sk-sub">{sec.sub.map((w, j) => <span key={j} className="set-sk-line"><Skeleton h={9} w={w} /></span>)}</div> : null}
            <div className="set-list">
              {sec.rows.map((row, i) => {
                const blocks = (bs: Block[]) => bs.map(([w, h, r], j) => <Skeleton key={j} h={h} w={w} r={r} />);
                return (
                  <div key={i} className="set-row">
                    <div className="set-words">
                      <span className="set-sk-line"><Skeleton h={11} w={row.t} /></span>
                      {row.d.map((w, j) => <span key={j} className="set-sk-line"><Skeleton h={9} w={w} /></span>)}
                    </div>
                    <div className="set-control">
                      {row.c2 ? <div className="set-hours"><div className="set-times">{blocks(row.c)}</div><div className="set-times">{blocks(row.c2)}</div></div> : blocks(row.c)}
                    </div>
                  </div>
                );
              })}
            </div>
          </div>
        ))}
      </div>
      <div className="set-index" aria-hidden>
        {INDEX_WIDTHS.map((w, i) => <span key={i} className="set-sk-index"><Skeleton h={10} w={w} /></span>)}
      </div>
    </div>
  );
}
