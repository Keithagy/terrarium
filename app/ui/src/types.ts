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
