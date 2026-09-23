// Layered layout for one C4 diagram. Small graphs (a dozen boxes), so a plain
// Sugiyama: break cycles, longest-path layers (some pinned: people on top,
// outside systems at the bottom), barycenter ordering, then rows centred and
// nudged toward their neighbours. Edges are cubic curves between box edges;
// a back edge loops around the right side.

export interface LNode {
  id: string;
  w: number;
  h: number;
  /** Pin to a layer: 0 is the top row, -1 the bottom row. */
  pin?: number;
}

export interface LEdge {
  from: string;
  to: string;
}

export interface Placed {
  id: string;
  x: number;
  y: number;
  w: number;
  h: number;
  layer: number;
}

export interface Routed {
  from: string;
  to: string;
  d: string;
  /** Where a label sits. */
  mid: { x: number; y: number };
  /** Direction of the arrow's last segment, for orienting the head. */
  back: boolean;
}

export interface Layout {
  nodes: Placed[];
  edges: Routed[];
  width: number;
  height: number;
}

export interface LayoutOptions {
  hgap: number;
  vgap: number;
  pad: number;
}

const DEFAULTS: LayoutOptions = { hgap: 48, vgap: 96, pad: 24 };

export function layout(nodes: LNode[], edges: LEdge[], options: Partial<LayoutOptions> = {}): Layout {
  const o = { ...DEFAULTS, ...options };
  if (nodes.length === 0) return { nodes: [], edges: [], width: 0, height: 0 };
  const index = new Map(nodes.map((n, i) => [n.id, i]));
  const n = nodes.length;
  const out: number[][] = nodes.map(() => []);
  const seenEdge = new Set<string>();
  for (const e of edges) {
    const a = index.get(e.from);
    const b = index.get(e.to);
    if (a === undefined || b === undefined || a === b) continue;
    const key = `${a}>${b}`;
    if (seenEdge.has(key)) continue;
    seenEdge.add(key);
    out[a].push(b);
  }

  // ---- break cycles: edges that close a DFS cycle are reversed for layering only
  const state = new Array<number>(n).fill(0);
  const fwd: number[][] = nodes.map(() => []);
  const dfs = (u: number) => {
    state[u] = 1;
    for (const v of out[u]) {
      if (state[v] === 1) fwd[v].push(u);
      else {
        if (state[v] === 0) dfs(v);
        fwd[u].push(v);
      }
    }
    state[u] = 2;
  };
  const order = [...nodes.keys()].sort((a, b) => (nodes[a].pin ?? 1) - (nodes[b].pin ?? 1) || out[b].length - out[a].length);
  for (const u of order) if (state[u] === 0) dfs(u);

  // ---- layers: longest path from the top, with pins
  const indeg = new Array<number>(n).fill(0);
  for (let u = 0; u < n; u++) for (const v of fwd[u]) indeg[v]++;
  const layer = new Array<number>(n).fill(0);
  const queue = [...nodes.keys()].filter((u) => indeg[u] === 0);
  const topo: number[] = [];
  while (queue.length) {
    const u = queue.shift()!;
    topo.push(u);
    for (const v of fwd[u]) {
      layer[v] = Math.max(layer[v], layer[u] + 1);
      if (--indeg[v] === 0) queue.push(v);
    }
  }
  // people (pin 0) sit above everything; anything they point at moves down if needed
  for (let u = 0; u < n; u++) if (nodes[u].pin === 0) layer[u] = 0;
  const free = [...nodes.keys()].filter((u) => nodes[u].pin === undefined);
  if (nodes.some((x) => x.pin === 0) && free.length) {
    const minFree = Math.min(...free.map((u) => layer[u]));
    if (minFree === 0) for (const u of free) layer[u] += 1;
  }
  const maxFree = free.length ? Math.max(...free.map((u) => layer[u])) : 0;
  for (let u = 0; u < n; u++) if (nodes[u].pin === -1) layer[u] = maxFree + 1;
  // compact: drop empty layers
  const used = [...new Set(layer)].sort((a, b) => a - b);
  const remap = new Map(used.map((l, i) => [l, i]));
  for (let u = 0; u < n; u++) layer[u] = remap.get(layer[u])!;
  const L = used.length;
  const rows: number[][] = Array.from({ length: L }, () => []);
  for (const u of topo) rows[layer[u]].push(u);
  for (let u = 0; u < n; u++) if (!topo.includes(u)) rows[layer[u]].push(u);

  // ---- ordering: barycenter sweeps
  const pos = new Array<number>(n).fill(0);
  const setPos = () => rows.forEach((r) => r.forEach((u, i) => (pos[u] = i)));
  setPos();
  const inn: number[][] = nodes.map(() => []);
  for (let u = 0; u < n; u++) for (const v of fwd[u]) inn[v].push(u);
  for (let sweep = 0; sweep < 6; sweep++) {
    const down = sweep % 2 === 0;
    for (let l = down ? 1 : L - 2; down ? l < L : l >= 0; down ? l++ : l--) {
      const bary = new Map<number, number>();
      for (const u of rows[l]) {
        const nb = down ? inn[u] : fwd[u];
        bary.set(u, nb.length ? nb.reduce((s, v) => s + pos[v], 0) / nb.length : pos[u]);
      }
      rows[l].sort((a, b) => bary.get(a)! - bary.get(b)! || pos[a] - pos[b]);
      setPos();
    }
  }

  // ---- coordinates: rows centred on the widest row, then pulled toward neighbours
  const rowW = rows.map((r) => r.reduce((s, u) => s + nodes[u].w, 0) + o.hgap * Math.max(0, r.length - 1));
  const width = Math.max(...rowW) + o.pad * 2;
  const x = new Array<number>(n).fill(0);
  rows.forEach((r, l) => {
    let cx = (width - rowW[l]) / 2;
    for (const u of r) {
      x[u] = cx;
      cx += nodes[u].w + o.hgap;
    }
  });
  const center = (u: number) => x[u] + nodes[u].w / 2;
  for (let pass = 0; pass < 8; pass++) {
    for (let l = 0; l < L; l++) {
      const r = rows[l];
      if (r.length === 0) continue;
      const want = r.map((u) => {
        const nb = [...inn[u], ...fwd[u]];
        return nb.length ? nb.reduce((s, v) => s + center(v), 0) / nb.length - nodes[u].w / 2 : x[u];
      });
      // move toward the wanted x, then push apart left to right, keeping within the plate
      r.forEach((u, i) => (x[u] = x[u] * 0.35 + want[i] * 0.65));
      for (let i = 1; i < r.length; i++) {
        const prev = r[i - 1];
        const minX = x[prev] + nodes[prev].w + o.hgap;
        if (x[r[i]] < minX) x[r[i]] = minX;
      }
      for (let i = r.length - 2; i >= 0; i--) {
        const next = r[i + 1];
        const maxX = x[next] - o.hgap - nodes[r[i]].w;
        if (x[r[i]] > maxX) x[r[i]] = maxX;
      }
    }
  }
  const minX = Math.min(...x);
  const maxX = Math.max(...[...nodes.keys()].map((u) => x[u] + nodes[u].w));
  const totalW = maxX - minX + o.pad * 2;
  const rowH = rows.map((r) => Math.max(0, ...r.map((u) => nodes[u].h)));
  const y: number[] = new Array(n).fill(0);
  let cy = o.pad;
  rows.forEach((r, l) => {
    for (const u of r) y[u] = cy + (rowH[l] - nodes[u].h) / 2;
    cy += rowH[l] + o.vgap;
  });
  const height = cy - o.vgap + o.pad;
  const placed: Placed[] = nodes.map((nd, u) => ({ id: nd.id, x: x[u] - minX + o.pad, y: y[u], w: nd.w, h: nd.h, layer: layer[u] }));
  const byId = new Map(placed.map((p) => [p.id, p]));

  // ---- edges
  const pairs = new Map<string, number>();
  for (const e of edges) pairs.set(`${e.from}>${e.to}`, (pairs.get(`${e.from}>${e.to}`) ?? 0) + 1);
  const routed: Routed[] = [];
  const seen = new Set<string>();
  // fan out edges leaving or entering the same side of a box so they do not overlap
  const outCount = new Map<string, number>();
  const inCount = new Map<string, number>();
  for (const e of edges) {
    if (!byId.has(e.from) || !byId.has(e.to) || e.from === e.to) continue;
    outCount.set(e.from, (outCount.get(e.from) ?? 0) + 1);
    inCount.set(e.to, (inCount.get(e.to) ?? 0) + 1);
  }
  const outSeen = new Map<string, number>();
  const inSeen = new Map<string, number>();
  const slot = (id: string, count: Map<string, number>, seenMap: Map<string, number>, w: number): [number, number] => {
    const total = count.get(id) ?? 1;
    const i = seenMap.get(id) ?? 0;
    seenMap.set(id, i + 1);
    const span = Math.min(w - 40, (total - 1) * 22);
    // how far down the row gap the label sits: a fan-out spreads its labels out
    const spacing = total <= 1 ? 0 : Math.min(0.27, 0.65 / (total - 1));
    const frac = Math.max(0.2, Math.min(0.85, 0.55 + (i - (total - 1) / 2) * spacing));
    return [total <= 1 ? 0 : -span / 2 + (span * i) / (total - 1), frac];
  };
  const sorted = [...edges].filter((e) => byId.has(e.from) && byId.has(e.to) && e.from !== e.to).sort((a, b) => byId.get(a.to)!.x - byId.get(b.to)!.x || byId.get(a.from)!.x - byId.get(b.from)!.x);
  for (const e of sorted) {
    const key = `${e.from}>${e.to}`;
    if (seen.has(key)) continue;
    seen.add(key);
    const a = byId.get(e.from)!;
    const b = byId.get(e.to)!;
    const both = pairs.has(`${e.to}>${e.from}`);
    const shift = both ? 14 : 0;
    if (b.layer > a.layer) {
      const [so, frac] = slot(e.from, outCount, outSeen, a.w);
      const [ti] = slot(e.to, inCount, inSeen, b.w);
      const sx = a.x + a.w / 2 + so + shift;
      const sy = a.y + a.h;
      const tx = b.x + b.w / 2 + ti + shift;
      const ty = b.y;
      const dy = Math.max(40, (ty - sy) * 0.5);
      // Labels live in the gap under the source row: short edges mid-gap, long edges
      // (which pass other rows) low in the gap, both staggered by fan-out.
      const labelY = sy + o.vgap * frac;
      routed.push({ from: e.from, to: e.to, d: `M${sx},${sy} C${sx},${sy + dy} ${tx},${ty - dy} ${tx},${ty}`, mid: cubicAtY(labelY, sx, sy, sx, sy + dy, tx, ty - dy, tx, ty), back: false });
    } else if (b.layer === a.layer) {
      const leftToRight = a.x < b.x;
      const sx = leftToRight ? a.x + a.w : a.x;
      const tx = leftToRight ? b.x : b.x + b.w;
      const sy = a.y + a.h / 2 - shift;
      const ty = b.y + b.h / 2 - shift;
      const dx = Math.max(30, Math.abs(tx - sx) * 0.4) * (leftToRight ? 1 : -1);
      routed.push({ from: e.from, to: e.to, d: `M${sx},${sy} C${sx + dx},${sy} ${tx - dx},${ty} ${tx},${ty}`, mid: cubicAt(0.5, sx, sy, sx + dx, sy, tx - dx, ty, tx, ty), back: false });
    } else {
      // a back edge: leave the right side, come back in on the right side of the target
      const sx = a.x + a.w;
      const sy = a.y + a.h / 2 + shift;
      const tx = b.x + b.w;
      const ty = b.y + b.h / 2 + shift;
      const reach = Math.max(sx, tx) + 56;
      routed.push({ from: e.from, to: e.to, d: `M${sx},${sy} C${reach},${sy} ${reach},${ty} ${tx},${ty}`, mid: cubicAt(0.5, sx, sy, reach, sy, reach, ty, tx, ty), back: true });
    }
  }
  return { nodes: placed, edges: routed, width: Math.max(totalW, width), height };
}

/** The point on a cubic whose y is closest to `y` (y is monotonic for these curves). */
function cubicAtY(y: number, x0: number, y0: number, x1: number, y1: number, x2: number, y2: number, x3: number, y3: number): { x: number; y: number } {
  let lo = 0;
  let hi = 1;
  for (let i = 0; i < 24; i++) {
    const mid = (lo + hi) / 2;
    if (cubicAt(mid, x0, y0, x1, y1, x2, y2, x3, y3).y < y) lo = mid; else hi = mid;
  }
  return cubicAt((lo + hi) / 2, x0, y0, x1, y1, x2, y2, x3, y3);
}

function cubicAt(t: number, x0: number, y0: number, x1: number, y1: number, x2: number, y2: number, x3: number, y3: number): { x: number; y: number } {
  const mt = 1 - t;
  return {
    x: mt * mt * mt * x0 + 3 * mt * mt * t * x1 + 3 * mt * t * t * x2 + t * t * t * x3,
    y: mt * mt * mt * y0 + 3 * mt * mt * t * y1 + 3 * mt * t * t * y2 + t * t * t * y3,
  };
}
