export type Lang = "rust" | "typescript" | "javascript" | "python" | "go" | "other";
export type NodeKind = "repo" | "package" | "file" | "symbol";
export type EdgeKind = "contains" | "imports" | "calls" | "flow";
export type Level = "package" | "file" | "symbol";

export interface ViewNode {
  id: number;
  name: string;
  path: string;
  kind: NodeKind;
  lang: Lang;
  loc: number;
  group: number;
  group_name: string;
  tags?: string[];
  external?: boolean;
  degree: number;
}

export interface ViewEdge {
  from: number;
  to: number;
  kind: EdgeKind;
  weight: number;
  labels?: string[];
}

export interface ViewGraph {
  level: NodeKind;
  focus?: number;
  nodes: ViewNode[];
  edges: ViewEdge[];
}

export interface Stats {
  files: number;
  symbols: number;
  packages: number;
  external_packages: number;
  imports: number;
  calls: number;
  flows: number;
  unresolved_calls: number;
  loc: number;
  by_lang: { lang: string; files: number; loc: number }[];
  scan_ms: number;
}

export interface ViewPayload {
  generation: number;
  level: Level;
  focus: number | null;
  view: ViewGraph;
  positions: number[];
  root: string;
  stats: Stats;
}

export interface GraphNode {
  id: number;
  kind: NodeKind;
  name: string;
  path: string;
  lang: Lang;
  parent: number | null;
  loc: number;
  symbol_kind?: string;
  span?: [number, number];
  tags?: string[];
  external?: boolean;
}

export interface Neighbour {
  id: number;
  name: string;
  path: string;
  kind: NodeKind;
  edge: EdgeKind;
  direction: "in" | "out";
  label?: string;
  weight: number;
}

export interface NodeDetail {
  node: GraphNode;
  neighbours: Neighbour[];
  children: { id: number; name: string; kind: NodeKind; lang: Lang; loc: number; tags: string[] }[];
  package: string | null;
  file: string | null;
}

export interface FlowRow {
  from: number;
  from_path: string;
  from_lang: Lang;
  to: number;
  to_path: string;
  to_lang: Lang;
  label: string;
}

// ---- the build: crates/terrarium-core/src/build.rs, serialised as is ----------

export interface SubBuild {
  id: string;
  name: string;
  blurb: string;
  package: string;
}

export interface BuildStep {
  sub_build: string;
  title: string;
  caption: string;
  files: string[];
}

export interface Design {
  schema: number;
  source: string; // "engine" | "claude"
  model?: string;
  scanned_at: string;
  title: string;
  summary: string;
  sub_builds: SubBuild[];
  steps: BuildStep[];
}

export interface WeakJoint {
  kind: "early" | "missing" | "duplicate" | "unknown" | "sub-build";
  file: string;
  detail: string;
}

export interface BuildCheck {
  ok: boolean;
  pieces: number;
  files: number;
  steps: number;
  sub_builds: number;
  joints: number;
  bridges: number;
  weak: WeakJoint[];
  interlocked: string[][];
  loose: string[];
  unresolved: number;
  gaps: number;
  repairs?: string[];
}

/** A package on the baseplate. Units are studs; x runs right, z runs toward the viewer. */
export interface District {
  sub_build: string;
  name: string;
  lang: Lang;
  x: number;
  z: number;
  w: number;
  d: number;
}

/** A file: a stack of `layers` bricks with a w×d footprint. */
export interface Building {
  id: number;
  path: string;
  name: string;
  lang: Lang;
  district: number;
  x: number;
  z: number;
  w: number;
  d: number;
  layers: number;
  /** Zero-based manual step that adds it. */
  step: number;
  /** Starts a trace across a boundary: a landmark with a lamp on top. */
  lamp: boolean;
  /** Buildings this one rests on (imports or calls into). */
  rests_on: number[];
}

/** One layer of a building: one symbol (or several, when a file has more than 14). */
export interface Brick {
  building: number;
  nodes: number[];
  name: string;
  layer: number;
  kind?: string;
  sinks?: string[];
}

/** A cross-language flow: an amber bridge between two bricks (indices into `bricks`). */
export interface Bridge {
  from: number;
  to: number;
  label: string;
  step: number;
}

export interface BuildModel {
  studs: [number, number];
  districts: District[];
  buildings: Building[];
  bricks: Brick[];
  bridges: Bridge[];
}

export interface Build {
  design: Design;
  check: BuildCheck;
  model: BuildModel;
  stale?: string;
}

export interface TraceStep {
  id: number;
  name: string;
  path: string;
  lang: Lang;
  lane: string;
  depth: number;
  parent?: number;
  via?: "calls" | "flow";
  label?: string;
  sinks?: string[];
  line?: number;
  repeat?: boolean;
}

export interface Trace {
  entry: number;
  entry_path: string;
  name: string;
  hops: number;
  langs: Lang[];
  lanes: string[];
  via: string[];
  sinks: string[];
  truncated: boolean;
  steps: TraceStep[];
}

export interface EndpointRef {
  id: number;
  name: string;
  path: string;
  lang: Lang;
}

export interface Endpoint {
  key: string;
  kind: "http" | "ipc" | "queue";
  status: "ok" | "no-callers" | "no-handler";
  handlers: EndpointRef[];
  callers: EndpointRef[];
}

export interface Boundary {
  id: number;
  name: string;
  path: string;
  lang: Lang;
  tags: string[];
}

export interface LayoutTick {
  generation: number;
  iteration: number;
  energy: number;
  backend: string;
  done: boolean;
  positions: number[];
}

export interface ScanDone {
  root: string;
  stats: Stats;
  cached: string;
  from_cache: boolean;
}

export interface CacheEntry {
  root: string;
  file: string;
  scanned_at: string;
  files: number;
  symbols: number;
  flows: number;
}

export const LANG_COLORS: Record<Lang, [number, number, number]> = {
  rust: [0xe0 / 255, 0x90 / 255, 0x4a / 255],
  typescript: [0x6f / 255, 0xb3 / 255, 0xe0 / 255],
  javascript: [0xe6 / 255, 0xd2 / 255, 0x5a / 255],
  python: [0x8f / 255, 0xbf / 255, 0x6a / 255],
  go: [0x5e / 255, 0xd3 / 255, 0xc0 / 255],
  other: [0xa9 / 255, 0x9a / 255, 0xc9 / 255],
};

export const LANG_LABEL: Record<Lang, string> = {
  rust: "Rust",
  typescript: "TypeScript",
  javascript: "JavaScript",
  python: "Python",
  go: "Go",
  other: "Other",
};
