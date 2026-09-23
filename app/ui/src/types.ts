export type Lang = "rust" | "typescript" | "javascript" | "python" | "go" | "other";
export type NodeKind = "repo" | "package" | "file" | "symbol";
export type EdgeKind = "contains" | "imports" | "calls" | "flow";

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
  children: { id: number; name: string; kind: NodeKind; lang: Lang; loc: number; tags: string[]; symbol_kind?: string; span?: [number, number] }[];
  package: string | null;
  file: string | null;
}

// ---- the atlas: crates/terrarium-core/src/atlas.rs, serialised as is ----------

export interface System {
  name: string;
  purpose: string;
  summary: string;
}

export interface Person {
  id: string;
  name: string;
  description: string;
}

export type ExternalKind = "database" | "queue" | "filesystem" | "service" | "system";

export interface External {
  id: string;
  name: string;
  kind: ExternalKind;
  description: string;
}

export type ContainerKind = "web" | "desktop" | "service" | "worker" | "cli" | "library" | "tooling";

export interface Container {
  id: string;
  package: string;
  name: string;
  kind: ContainerKind;
  language?: string;
  technology: string;
  description: string;
  responsibilities?: string[];
  hidden?: boolean;
  components: Component[];
}

export interface Component {
  id: string;
  name: string;
  description: string;
  technology?: string;
  responsibilities?: string[];
  files: string[];
}

export interface Evidence {
  from: string;
  to: string;
  via: "imports" | "calls" | "flow" | "tag";
  label?: string;
  line?: number;
}

export interface Relationship {
  from: string;
  to: string;
  label: string;
  technology?: string;
  source: "code" | "survey" | "claimed";
  level: "container" | "component";
  evidence?: Evidence[];
}

export interface JourneyStep {
  from: string;
  to: string;
  from_path: string;
  to_path: string;
  label: string;
  caption: string;
}

export type MessageKind = "call" | "flow" | "store" | "return";
export type Author = "engine" | "claude" | "user";

/** One arrow on the sequence diagram, between two atlas element ids. */
export interface Message {
  from: string;
  to: string;
  label: string;
  caption: string;
  kind: MessageKind;
  depth: number;
  source: "code" | "survey" | "claimed" | "";
  by: Author | "";
  from_path?: string;
  to_path?: string;
}

export interface Journey {
  id: string;
  name: string;
  summary: string;
  entry: string;
  /** The source of truth: kept at the finest grain, projected onto each level. */
  messages: Message[];
  /** Component-grain steps for the map overlay; derived from `messages` by the check. */
  steps: JourneyStep[];
  source: Author | "";
  note?: string;
}

/** A flow the person asks the narrators to follow. */
export interface FlowRequest {
  entry: string;
  name: string;
  note: string;
}

export interface Pointer {
  element: string;
  why: string;
}

export interface Callout {
  title: string;
  detail: string;
  element?: string;
}

export interface Guide {
  start_here: Pointer[];
  callouts: Callout[];
}

export interface Report {
  backed: number;
  survey: number;
  claimed: number;
  unplaced?: string[];
  notes?: string[];
}

export interface Atlas {
  schema: number;
  source: "engine" | "claude";
  model?: string;
  scanned_at: string;
  system: System;
  people: Person[];
  externals: External[];
  containers: Container[];
  relationships: Relationship[];
  journeys: Journey[];
  guide: Guide;
  report: Report;
}

export interface AtlasView {
  atlas: Atlas;
  stale?: string;
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

// ---- discovery progress: crates/terrarium-core/src/discovery.rs -----------------

export type Progress =
  | { event: "started"; model: string; containers: number; journeys: number }
  | { event: "stage"; stage: "survey" | "field" | "editor" | "verify" }
  | { event: "agent_started"; role: string; target: string | null; name: string }
  | { event: "agent_activity"; role: string; target: string | null; kind: "reading" | "searching"; path?: string; query?: string }
  | { event: "agent_done"; role: string; target: string | null; ok: boolean; cost_usd: number; secs: number; error: string | null }
  | { event: "survey_done"; system: System; people: Person[]; externals: External[]; containers: Container[] }
  | { event: "container_done"; container: Container }
  | { event: "journey_done"; journey: Journey }
  | { event: "verified"; atlas: Atlas };

export type NarrateDone = { run: DiscoveryRun; journey: Journey | null };

export interface DiscoveryRun {
  model: string;
  agents: { role: string; target?: string; ok: boolean; error?: string; cost_usd: number; secs: number; reads: number }[];
  cost_usd: number;
  secs: number;
}

export const LANG_LABEL: Record<Lang, string> = {
  rust: "Rust",
  typescript: "TypeScript",
  javascript: "JavaScript",
  python: "Python",
  go: "Go",
  other: "Other",
};

export const KIND_LABEL: Record<ContainerKind, string> = {
  web: "Web app",
  desktop: "Desktop app",
  service: "Service",
  worker: "Worker",
  cli: "Command line",
  library: "Library",
  tooling: "Tooling",
};

export const EXTERNAL_LABEL: Record<ExternalKind, string> = {
  database: "Database",
  queue: "Queue",
  filesystem: "File system",
  service: "Outside service",
  system: "Outside system",
};

/** A container's colour: the language the engine measured, else the one its technology line starts with. */
export function langOf(technology: string, language?: string): Lang {
  if (language && language in LANG_LABEL) return language as Lang;
  const t = technology.toLowerCase();
  if (t.startsWith("rust")) return "rust";
  if (t.startsWith("typescript")) return "typescript";
  if (t.startsWith("javascript")) return "javascript";
  if (t.startsWith("python")) return "python";
  if (t.startsWith("go")) return "go";
  return "other";
}
