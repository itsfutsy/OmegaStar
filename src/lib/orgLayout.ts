// Tidy tree layout for the org chart. Pure: members in, boxes and connector paths out.
//
// A plain hierarchy at every size: a manager's reports sit side by side in one row under them, the manager centred
// over it, each report's own reports under them in turn. The one exception keeps a wide team narrow: a manager whose
// reports have none of their own stacks them in one column straight under them, once there are `stackMin` of them.
// Lines are 1px, orthogonal, with rounded elbows.
import { YOU_ID } from "./org";

/** The geometry of one size of chart: card width, the gaps, and the spine gutter beside a stacked column. */
export type ChartMetrics = { w: number; gapX: number; gapY: number; stackGap: number; gutter: number; indent: number };

export const WIDE: ChartMetrics = { w: 248, gapX: 40, gapY: 48, stackGap: 12, gutter: 14, indent: 30 };
export const COMPACT: ChartMetrics = { w: 208, gapX: 28, gapY: 28, stackGap: 10, gutter: 12, indent: 26 };

/** Every node, you (YOU_ID) included. `parentId` null: the top of the company. `h`: the card's height. */
type LayoutInput = { id: string; parentId: string | null; h: number }[];
type Box = { id: string; x: number; y: number; w: number; h: number; depth: number };
type Edge = { id: string; from: string; to: string; d: string };
type Layout = { boxes: Map<string, Box>; edges: Edge[]; width: number; height: number };

type Node = { id: string; h: number; children: Node[] };
type Pt = [number, number];
/** A laid-out subtree, relative to its own top-left. `attach` is where a line from above meets it. */
type Sub = { width: number; height: number; attach: number; boxes: Box[]; edges: { id: string; from: string; to: string; pts: Pt[] }[] };

const shift = (s: Sub, dx: number, dy: number): Sub => ({
  ...s,
  attach: s.attach + dx,
  boxes: s.boxes.map((b) => ({ ...b, x: b.x + dx, y: b.y + dy })),
  edges: s.edges.map((e) => ({ ...e, pts: e.pts.map(([x, y]) => [x + dx, y + dy] as Pt) })),
});

/** An orthogonal polyline as a path with rounded elbows. */
function rounded(pts: Pt[], r = 6): string {
  const clean = pts.filter((p, i) => i === 0 || p[0] !== pts[i - 1][0] || p[1] !== pts[i - 1][1]);
  if (clean.length < 2) return "";
  let d = `M${clean[0][0]} ${clean[0][1]}`;
  for (let i = 1; i < clean.length - 1; i++) {
    const [a, b, c] = [clean[i - 1], clean[i], clean[i + 1]];
    const inLen = Math.hypot(b[0] - a[0], b[1] - a[1]);
    const outLen = Math.hypot(c[0] - b[0], c[1] - b[1]);
    const k = Math.min(r, inLen / 2, outLen / 2);
    const p1: Pt = [b[0] - Math.sign(b[0] - a[0]) * k, b[1] - Math.sign(b[1] - a[1]) * k];
    const p2: Pt = [b[0] + Math.sign(c[0] - b[0]) * k, b[1] + Math.sign(c[1] - b[1]) * k];
    d += ` L${p1[0]} ${p1[1]} Q${b[0]} ${b[1]} ${p2[0]} ${p2[1]}`;
  }
  const last = clean[clean.length - 1];
  return `${d} L${last[0]} ${last[1]}`;
}

/** `stackMin`: how many reports, none with reports of their own, a manager has before they stack in a column under
 *  them rather than sit in a row. */
export function layoutOrg(nodes: LayoutInput, m: ChartMetrics, { stackMin = 3 }: { stackMin?: number } = {}): Layout {
  const heights = new Map(nodes.map((n) => [n.id, n.h]));
  const ids = new Set([YOU_ID, ...nodes.map((n) => n.id)]);
  const parent = new Map<string, string | null>([[YOU_ID, null]]);
  for (const n of nodes) parent.set(n.id, n.parentId !== null && ids.has(n.parentId) && n.parentId !== n.id ? n.parentId : null);
  const topOf = (id: string): string | null => {
    const seen = new Set<string>();
    let cur = id;
    for (let p = parent.get(cur) ?? null; p !== null; p = parent.get(cur) ?? null) {
      if (seen.has(p)) return null;
      seen.add(p);
      cur = p;
    }
    return cur;
  };
  // One tree, rooted at the top of your line. Anything that can't reach it hangs off you.
  const root = topOf(YOU_ID) ?? YOU_ID;
  if (root === YOU_ID) parent.set(YOU_ID, null);
  for (const id of ids) if (id !== root && topOf(id) !== root) parent.set(id, YOU_ID);
  const byParent = new Map<string, string[]>();
  for (const n of [{ id: YOU_ID }, ...nodes]) {
    const p = parent.get(n.id) ?? null;
    if (p !== null && !(byParent.get(p) ?? []).includes(n.id)) byParent.set(p, [...(byParent.get(p) ?? []), n.id]);
  }
  const build = (id: string): Node => ({ id, h: heights.get(id) ?? 60, children: (byParent.get(id) ?? []).map(build) });

  const lay = (n: Node, depth: number): Sub => {
    const self: Box = { id: n.id, x: 0, y: 0, w: m.w, h: n.h, depth };
    if (!n.children.length) return { width: m.w, height: n.h, attach: m.w / 2, boxes: [self], edges: [] };

    // Enough reports with none of their own: they stack straight under the manager, hung from a spine.
    if (n.children.length >= stackMin && n.children.every((c) => !c.children.length)) {
      const spine = m.indent - m.gutter;
      const boxes = [self];
      const edges: Sub["edges"] = [];
      let y = n.h + m.stackGap + 4;
      for (const c of n.children) {
        boxes.push({ id: c.id, x: m.indent, y, w: m.w, h: c.h, depth: depth + 1 });
        const mid = y + Math.min(c.h / 2, 24);
        edges.push({ id: `${n.id}>${c.id}`, from: n.id, to: c.id, pts: [[spine, n.h], [spine, mid], [m.indent, mid]] });
        y += c.h + m.stackGap;
      }
      return { width: m.indent + m.w, height: y - m.stackGap, attach: m.w / 2, boxes, edges };
    }

    // Otherwise one row of reports, each with their own team under them, the manager centred over the row. The row's
    // cards share one height, so their bottoms and the lines below them line up.
    const h = Math.max(...n.children.map((c) => c.h));
    for (const c of n.children) c.h = h;
    const top = n.h + m.gapY;
    const items: { id: string; sub: Sub; left: number }[] = [];
    let x = 0;
    for (const c of n.children) {
      const s = lay(c, depth + 1);
      items.push({ id: c.id, sub: s, left: x });
      x += s.width + m.gapX;
    }
    const rowRight = x - m.gapX;
    const first = items[0], last = items[items.length - 1];
    const centre = (first.left + first.sub.attach + last.left + last.sub.attach) / 2;
    const selfLeft = centre - m.w / 2;
    const minX = Math.min(0, selfLeft);
    const dx = -minX;
    const width = Math.max(rowRight, selfLeft + m.w) - minX;
    const bus = n.h + m.gapY / 2;
    const px = centre + dx;

    const boxes: Box[] = [{ ...self, x: selfLeft + dx }];
    const edges: Sub["edges"] = [];
    let height = n.h;
    for (const it of items) {
      const s = shift(it.sub, it.left + dx, top);
      boxes.push(...s.boxes);
      edges.push(...s.edges);
      edges.push({ id: `${n.id}>${it.id}`, from: n.id, to: it.id, pts: [[px, n.h], [px, bus], [s.attach, bus], [s.attach, top]] });
      height = Math.max(height, top + s.height);
    }
    return { width, height, attach: px, boxes, edges };
  };

  const sub = lay(build(root), 0);
  return {
    boxes: new Map(sub.boxes.map((b) => [b.id, b])),
    edges: sub.edges.map((e) => ({ id: e.id, from: e.from, to: e.to, d: rounded(e.pts) })),
    width: sub.width,
    height: sub.height,
  };
}

/** The members in tree order, each with its depth: the outline's rows. Same tree as `layoutOrg`. */
export function treeOrder(nodes: { id: string; parentId: string | null }[]): { id: string; depth: number; parentId: string | null }[] {
  const ids = new Set([YOU_ID, ...nodes.map((n) => n.id)]);
  const parent = new Map<string, string | null>([[YOU_ID, null]]);
  for (const n of nodes) parent.set(n.id, n.parentId !== null && ids.has(n.parentId) && n.parentId !== n.id ? n.parentId : null);
  const topOf = (id: string): string | null => {
    const seen = new Set<string>();
    let cur = id;
    for (let p = parent.get(cur) ?? null; p !== null; p = parent.get(cur) ?? null) {
      if (seen.has(p)) return null;
      seen.add(p);
      cur = p;
    }
    return cur;
  };
  const root = topOf(YOU_ID) ?? YOU_ID;
  if (root === YOU_ID) parent.set(YOU_ID, null);
  for (const id of ids) if (id !== root && topOf(id) !== root) parent.set(id, YOU_ID);
  const kids = new Map<string, string[]>();
  for (const n of [{ id: YOU_ID }, ...nodes]) {
    const p = parent.get(n.id) ?? null;
    if (p !== null && !(kids.get(p) ?? []).includes(n.id)) kids.set(p, [...(kids.get(p) ?? []), n.id]);
  }
  const out: { id: string; depth: number; parentId: string | null }[] = [];
  const walk = (id: string, depth: number) => {
    out.push({ id, depth, parentId: parent.get(id) ?? null });
    for (const c of kids.get(id) ?? []) walk(c, depth + 1);
  };
  walk(root, 0);
  return out;
}
