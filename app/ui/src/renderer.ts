// WebGL2 renderer: instanced nodes (SDF discs with glow), instanced edges
// (AA quads, brighter toward the target), soft package patches underneath,
// and a 2D canvas overlay for labels. GPU does all the per-node work; the CPU
// only rebuilds instance buffers when the graph, filters or positions change.

import { store, nodeVisible, subscribe, emit } from "./store";
import { LANG_COLORS, type ViewNode } from "./types";
import { log } from "./tauri";

const VS_NODE = `#version 300 es
precision highp float;
layout(location=0) in vec2 corner;
layout(location=1) in vec2 a_pos;
layout(location=2) in float a_radius;
layout(location=3) in vec3 a_color;
layout(location=4) in float a_state;
layout(location=5) in float a_flags;  // 1 = external dependency (drawn hollow)
uniform vec4 u_view;   // cx, cy, zoom, dpr
uniform vec2 u_res;    // css px
out vec2 v_uv;
out vec3 v_color;
out float v_state;
out float v_px;
out float v_flags;
void main() {
  float zoom = u_view.z;
  float r = clamp(a_radius * zoom, 2.5, 64.0);
  float margin = (a_state == 2.0 || a_state == 1.0) ? 3.2 : 1.6;
  v_px = r;
  vec2 screen = (a_pos - u_view.xy) * zoom + u_res * 0.5;
  vec2 p = screen + corner * r * margin;
  vec2 clip = (p / u_res) * 2.0 - 1.0;
  gl_Position = vec4(clip.x, -clip.y, 0.0, 1.0);
  v_uv = corner * margin;
  v_color = a_color;
  v_state = a_state;
  v_flags = a_flags;
}`;

const FS_NODE = `#version 300 es
precision highp float;
in vec2 v_uv;
in vec3 v_color;
in float v_state;
in float v_px;
in float v_flags;
out vec4 frag;
void main() {
  float d = length(v_uv);
  float aa = 1.2 / v_px;
  float core = 1.0 - smoothstep(1.0 - aa, 1.0 + aa, d);
  vec3 col = v_color;
  float alpha = core;
  if (v_flags > 0.5) {
    // external dependency: a hollow ring with a faint fill, so "lives outside the repo" is visible at a glance
    float t = clamp(1.6 / v_px, 0.12, 0.3);
    float ring = smoothstep(1.0 - t - aa, 1.0 - t + aa, d) * core;
    alpha = max(ring * 0.95, core * 0.10);
  } else {
    // inner highlight: a little lamp-light on the top-left of every disc
    float hi = smoothstep(0.9, 0.0, length(v_uv - vec2(-0.35, -0.35))) * 0.25;
    col += hi;
  }
  if (v_state == 3.0) { alpha *= 0.22; col = mix(col, vec3(0.35, 0.42, 0.38), 0.5); }
  if (v_state == 4.0) { col = mix(col, vec3(1.0), 0.12); }
  float glow = 0.0;
  if (v_state == 2.0) { glow = exp(-d * 1.6) * 0.75 * step(1.0, d); col = mix(col, vec3(0.95, 0.73, 0.31), step(1.0, d)); float ring = 1.0 - smoothstep(0.04, 0.10, abs(d - 1.25)); alpha = max(alpha, ring * 0.9); }
  if (v_state == 1.0) { glow = exp(-d * 2.2) * 0.45 * step(1.0, d); float ring = 1.0 - smoothstep(0.04, 0.10, abs(d - 1.18)); alpha = max(alpha, ring * 0.7); col = mix(col, vec3(0.85, 0.89, 0.85), ring * 0.6); }
  alpha = max(alpha, glow);
  if (alpha < 0.003) discard;
  frag = vec4(col * alpha, alpha);
}`;

const VS_EDGE = `#version 300 es
precision highp float;
layout(location=0) in vec2 corner;   // x: 0..1 along, y: -1..1 across
layout(location=1) in vec2 a_a;
layout(location=2) in vec2 a_b;
layout(location=3) in vec4 a_color;
layout(location=4) in float a_width;
layout(location=5) in float a_state;
uniform vec4 u_view;
uniform vec2 u_res;
out vec2 v_ac;   // along (0..1), across (-1..1)
out vec4 v_color;
out float v_state;
out float v_wpx;
void main() {
  float zoom = u_view.z;
  vec2 sa = (a_a - u_view.xy) * zoom + u_res * 0.5;
  vec2 sb = (a_b - u_view.xy) * zoom + u_res * 0.5;
  vec2 dir = sb - sa;
  float len = max(length(dir), 0.001);
  vec2 n = vec2(-dir.y, dir.x) / len;
  float w = max(a_width * sqrt(zoom), 0.8) + 1.5;
  vec2 p = sa + dir * corner.x + n * corner.y * w;
  vec2 clip = (p / u_res) * 2.0 - 1.0;
  gl_Position = vec4(clip.x, -clip.y, 0.0, 1.0);
  v_ac = corner;
  v_color = a_color;
  v_state = a_state;
  v_wpx = w;
}`;

const FS_EDGE = `#version 300 es
precision highp float;
in vec2 v_ac;
in vec4 v_color;
in float v_state;
in float v_wpx;
uniform float u_time;
out vec4 frag;
void main() {
  float across = abs(v_ac.y) * v_wpx;
  float edge = v_wpx - 1.5;
  float a = 1.0 - smoothstep(edge - 0.8, edge + 0.8, across);
  // fade toward the source so direction reads without arrowheads
  float along = mix(0.35, 1.0, v_ac.x);
  float alpha = v_color.a * a * along;
  vec3 col = v_color.rgb;
  if (v_state == 3.0) alpha *= 0.15;
  if (v_state == 1.0) {
    // highlighted: a slow pulse of light travelling toward the target
    float pulse = 0.5 + 0.5 * sin((v_ac.x * 14.0) - u_time * 2.2);
    col += pulse * 0.35;
    alpha = min(1.0, alpha * 1.35 + pulse * 0.15 * a);
  }
  if (alpha < 0.003) discard;
  frag = vec4(col * alpha, alpha);
}`;

const VS_PATCH = `#version 300 es
precision highp float;
layout(location=0) in vec2 corner;
layout(location=1) in vec2 a_pos;
layout(location=2) in float a_radius;
layout(location=3) in vec3 a_color;
uniform vec4 u_view;
uniform vec2 u_res;
out vec2 v_uv;
out vec3 v_color;
void main() {
  vec2 screen = (a_pos - u_view.xy) * u_view.z + u_res * 0.5;
  vec2 p = screen + corner * a_radius * u_view.z;
  vec2 clip = (p / u_res) * 2.0 - 1.0;
  gl_Position = vec4(clip.x, -clip.y, 0.0, 1.0);
  v_uv = corner;
  v_color = a_color;
}`;

const FS_PATCH = `#version 300 es
precision highp float;
in vec2 v_uv;
in vec3 v_color;
out vec4 frag;
void main() {
  float d = length(v_uv);
  float a = (1.0 - smoothstep(0.55, 1.0, d)) * 0.10;
  if (a < 0.002) discard;
  frag = vec4(v_color * a, a);
}`;

function compile(gl: WebGL2RenderingContext, type: number, src: string): WebGLShader {
  const s = gl.createShader(type)!;
  gl.shaderSource(s, src);
  gl.compileShader(s);
  if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) throw new Error(`shader: ${gl.getShaderInfoLog(s)}`);
  return s;
}

function program(gl: WebGL2RenderingContext, vs: string, fs: string): WebGLProgram {
  const p = gl.createProgram()!;
  gl.attachShader(p, compile(gl, gl.VERTEX_SHADER, vs));
  gl.attachShader(p, compile(gl, gl.FRAGMENT_SHADER, fs));
  gl.linkProgram(p);
  if (!gl.getProgramParameter(p, gl.LINK_STATUS)) throw new Error(`link: ${gl.getProgramInfoLog(p)}`);
  return p;
}

export interface FrameStats {
  fps: number;
  frame_ms_p50: number;
  frame_ms_p95: number;
  draw_calls: number;
  nodes_drawn: number;
  edges_drawn: number;
  labels_drawn: number;
  renderer: string;
}

const EDGE_COLORS: Record<string, [number, number, number, number]> = {
  imports: [0.56, 0.64, 0.59, 0.45],
  calls: [0.85, 0.89, 0.85, 0.32],
  flow: [0.95, 0.73, 0.31, 0.95],
  contains: [0.4, 0.4, 0.4, 0.2],
};

export class Renderer {
  gl: WebGL2RenderingContext;
  labels: CanvasRenderingContext2D;
  private nodeProg: WebGLProgram;
  private edgeProg: WebGLProgram;
  private patchProg: WebGLProgram;
  private quad: WebGLBuffer;
  private quadEdge: WebGLBuffer;
  private nodeVao: WebGLVertexArrayObject;
  private edgeVao: WebGLVertexArrayObject;
  private patchVao: WebGLVertexArrayObject;
  private nodeBuf: WebGLBuffer;
  private edgeBuf: WebGLBuffer;
  private patchBuf: WebGLBuffer;
  private nodeCount = 0;
  private edgeCount = 0;
  private patchCount = 0;
  private nodeData = new Float32Array(0); // pos(2) radius(1) color(3) state(1) flags(1) = 8
  private edgeData = new Float32Array(0); // a(2) b(2) color(4) width(1) state(1) = 10
  private patchData = new Float32Array(0); // pos(2) r(1) color(3) = 6
  private edgePairs: Int32Array = new Int32Array(0); // node indices per drawn edge (a,b)
  private visible = new Uint8Array(0);
  private radii = new Float32Array(0);
  private dirtyGeometry = true;
  private dirtyState = true;
  private dirtyPositions = true;
  private frameTimes: number[] = [];
  private lastFrame = 0;
  private drawCalls = 0;
  private labelsDrawn = 0;
  private lastRenderAt = 0;
  private needsRender = true;
  private rafId = 0;
  width = 0;
  height = 0;
  dpr = 1;
  time = 0;

  constructor(private canvas: HTMLCanvasElement, labelCanvas: HTMLCanvasElement) {
    const gl = canvas.getContext("webgl2", { antialias: true, alpha: true, premultipliedAlpha: true, preserveDrawingBuffer: true, powerPreference: "high-performance" });
    if (!gl) throw new Error("WebGL2 is not available in this webview");
    this.gl = gl;
    this.labels = labelCanvas.getContext("2d")!;
    this.nodeProg = program(gl, VS_NODE, FS_NODE);
    this.edgeProg = program(gl, VS_EDGE, FS_EDGE);
    this.patchProg = program(gl, VS_PATCH, FS_PATCH);
    this.quad = gl.createBuffer()!;
    gl.bindBuffer(gl.ARRAY_BUFFER, this.quad);
    gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 1, -1, -1, 1, 1, 1]), gl.STATIC_DRAW);
    this.quadEdge = gl.createBuffer()!;
    gl.bindBuffer(gl.ARRAY_BUFFER, this.quadEdge);
    gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([0, -1, 1, -1, 0, 1, 1, 1]), gl.STATIC_DRAW);
    this.nodeBuf = gl.createBuffer()!;
    this.edgeBuf = gl.createBuffer()!;
    this.patchBuf = gl.createBuffer()!;
    this.nodeVao = this.makeVao(this.quad, this.nodeBuf, [[1, 2], [2, 1], [3, 3], [4, 1], [5, 1]], 8);
    this.edgeVao = this.makeVao(this.quadEdge, this.edgeBuf, [[1, 2], [2, 2], [3, 4], [4, 1], [5, 1]], 10);
    this.patchVao = this.makeVao(this.quad, this.patchBuf, [[1, 2], [2, 1], [3, 3]], 6);
    gl.enable(gl.BLEND);
    gl.blendFunc(gl.ONE, gl.ONE_MINUS_SRC_ALPHA);
    const dbg = gl.getExtension("WEBGL_debug_renderer_info");
    this.rendererName = dbg ? String(gl.getParameter(dbg.UNMASKED_RENDERER_WEBGL)) : String(gl.getParameter(gl.RENDERER));
    log("info", "renderer ready", { renderer: this.rendererName });
    subscribe("graph", () => { this.dirtyGeometry = true; this.requestRender(); });
    subscribe("filters", () => { this.dirtyGeometry = true; this.requestRender(); });
    subscribe("positions", () => { this.dirtyPositions = true; this.requestRender(); });
    subscribe("selection", () => { this.dirtyState = true; this.requestRender(); });
    subscribe("hover", () => { this.dirtyState = true; this.requestRender(); });
    subscribe("camera", () => this.requestRender());
    subscribe("layout", () => this.requestRender());
    this.resize();
    window.addEventListener("resize", () => { this.resize(); this.requestRender(); });
    this.loop(performance.now());
  }

  rendererName: string;

  private makeVao(quad: WebGLBuffer, inst: WebGLBuffer, attrs: [number, number][], stride: number): WebGLVertexArrayObject {
    const gl = this.gl;
    const vao = gl.createVertexArray()!;
    gl.bindVertexArray(vao);
    gl.bindBuffer(gl.ARRAY_BUFFER, quad);
    gl.enableVertexAttribArray(0);
    gl.vertexAttribPointer(0, 2, gl.FLOAT, false, 0, 0);
    gl.bindBuffer(gl.ARRAY_BUFFER, inst);
    let offset = 0;
    for (const [loc, size] of attrs) {
      gl.enableVertexAttribArray(loc);
      gl.vertexAttribPointer(loc, size, gl.FLOAT, false, stride * 4, offset * 4);
      gl.vertexAttribDivisor(loc, 1);
      offset += size;
    }
    gl.bindVertexArray(null);
    return vao;
  }

  resize(): void {
    this.dpr = Math.min(window.devicePixelRatio || 1, 2);
    this.width = window.innerWidth;
    this.height = window.innerHeight;
    for (const c of [this.canvas, this.labels.canvas]) {
      c.width = Math.floor(this.width * this.dpr);
      c.height = Math.floor(this.height * this.dpr);
      c.style.width = `${this.width}px`;
      c.style.height = `${this.height}px`;
    }
    this.gl.viewport(0, 0, this.canvas.width, this.canvas.height);
  }

  requestRender(): void {
    this.needsRender = true;
  }

  private loop(now: number): void {
    this.rafId = requestAnimationFrame((t) => this.loop(t));
    this.time = now / 1000;
    const animate = store.layout.running || (store.selection !== null && store.edges.some((e) => e.kind === "flow" && (e.from === store.selection || e.to === store.selection)));
    if (!this.needsRender && !animate) return;
    this.needsRender = false;
    const t0 = performance.now();
    this.render();
    const dt = performance.now() - t0;
    this.frameTimes.push(dt);
    if (this.frameTimes.length > 120) this.frameTimes.shift();
    this.lastFrame = now;
    this.lastRenderAt = now;
  }

  destroy(): void {
    cancelAnimationFrame(this.rafId);
  }

  // ---- geometry ----------------------------------------------------------

  radiusFor(n: ViewNode): number {
    if (n.external) return 4.2 + Math.min(Math.sqrt(n.degree) * 0.6, 4);
    const base = n.kind === "package" ? 9 : n.kind === "file" ? 5 : 3.2;
    return base + Math.min(Math.log1p(n.loc) * 0.9, 9) * (n.kind === "symbol" ? 0.5 : 1) + Math.sqrt(n.degree) * 0.6;
  }

  private rebuildGeometry(): void {
    const nodes = store.nodes;
    const n = nodes.length;
    this.visible = new Uint8Array(n);
    this.radii = new Float32Array(n);
    this.nodeData = new Float32Array(n * 8);
    for (let i = 0; i < n; i++) {
      const node = nodes[i];
      this.visible[i] = nodeVisible(node) ? 1 : 0;
      this.radii[i] = this.radiusFor(node);
      const c = LANG_COLORS[node.lang] ?? LANG_COLORS.other;
      const o = i * 8;
      this.nodeData[o + 2] = this.radii[i];
      this.nodeData[o + 3] = c[0];
      this.nodeData[o + 4] = c[1];
      this.nodeData[o + 5] = c[2];
      this.nodeData[o + 7] = node.external ? 1 : 0;
    }
    // edges
    const pairs: number[] = [];
    const kinds: string[] = [];
    for (const e of store.edges) {
      if (!store.filters.edges.has(e.kind)) continue;
      const a = store.index.get(e.from);
      const b = store.index.get(e.to);
      if (a === undefined || b === undefined) continue;
      if (!this.visible[a] || !this.visible[b]) continue;
      pairs.push(a, b);
      kinds.push(e.kind);
    }
    this.edgePairs = Int32Array.from(pairs);
    this.edgeCount = kinds.length;
    this.edgeData = new Float32Array(this.edgeCount * 10);
    let k = 0;
    for (const e of store.edges) {
      if (!store.filters.edges.has(e.kind)) continue;
      const a = store.index.get(e.from);
      const b = store.index.get(e.to);
      if (a === undefined || b === undefined || !this.visible[a] || !this.visible[b]) continue;
      const c = EDGE_COLORS[e.kind];
      const o = k * 10;
      this.edgeData[o + 4] = c[0];
      this.edgeData[o + 5] = c[1];
      this.edgeData[o + 6] = c[2];
      this.edgeData[o + 7] = c[3] * (e.kind === "flow" ? 1 : Math.min(1, 0.6 + Math.log1p(e.weight) * 0.2));
      this.edgeData[o + 8] = e.kind === "flow" ? 2.2 + Math.min(e.weight, 4) * 0.3 : 0.9 + Math.min(Math.log1p(e.weight) * 0.5, 1.5);
      k++;
    }
    // packages (groups)
    const groups = new Map<number, number[]>();
    for (let i = 0; i < n; i++) {
      if (!this.visible[i]) continue;
      const g = nodes[i].group;
      if (!groups.has(g)) groups.set(g, []);
      groups.get(g)!.push(i);
    }
    this.patchGroups = [...groups.values()].filter((g) => g.length > 1);
    this.patchCount = this.patchGroups.length;
    this.patchData = new Float32Array(this.patchCount * 6);
    this.nodeCount = n;
    this.dirtyGeometry = false;
    this.dirtyPositions = true;
    this.dirtyState = true;
  }

  private patchGroups: number[][] = [];

  private updatePositions(): void {
    const pos = store.positions;
    const n = this.nodeCount;
    for (let i = 0; i < n; i++) {
      this.nodeData[i * 8] = pos[i * 2];
      this.nodeData[i * 8 + 1] = pos[i * 2 + 1];
    }
    for (let k = 0; k < this.edgeCount; k++) {
      const a = this.edgePairs[k * 2];
      const b = this.edgePairs[k * 2 + 1];
      const o = k * 10;
      this.edgeData[o] = pos[a * 2];
      this.edgeData[o + 1] = pos[a * 2 + 1];
      this.edgeData[o + 2] = pos[b * 2];
      this.edgeData[o + 3] = pos[b * 2 + 1];
    }
    for (let g = 0; g < this.patchCount; g++) {
      const members = this.patchGroups[g];
      let cx = 0, cy = 0;
      for (const i of members) { cx += pos[i * 2]; cy += pos[i * 2 + 1]; }
      cx /= members.length; cy /= members.length;
      let r = 0;
      const colour = [0, 0, 0];
      for (const i of members) {
        const d = Math.hypot(pos[i * 2] - cx, pos[i * 2 + 1] - cy) + this.radii[i];
        if (d > r) r = d;
        const c = LANG_COLORS[store.nodes[i].lang];
        colour[0] += c[0]; colour[1] += c[1]; colour[2] += c[2];
      }
      const o = g * 6;
      this.patchData[o] = cx; this.patchData[o + 1] = cy; this.patchData[o + 2] = r + 26;
      this.patchData[o + 3] = colour[0] / members.length; this.patchData[o + 4] = colour[1] / members.length; this.patchData[o + 5] = colour[2] / members.length;
    }
    this.dirtyPositions = false;
    this.gridDirty = true;
  }

  private updateState(): void {
    const sel = store.selection;
    const hov = store.hover;
    const hasSel = sel !== null;
    for (let i = 0; i < this.nodeCount; i++) {
      const id = store.nodes[i].id;
      let s = 0;
      if (hasSel) s = id === sel ? 2 : store.neighbourIds.has(id) ? 4 : 3;
      if (id === hov && id !== sel) s = 1;
      if (!this.visible[i]) s = 5; // hidden: radius 0 below
      this.nodeData[i * 8 + 6] = s;
      if (s === 5) this.nodeData[i * 8 + 2] = 0; else this.nodeData[i * 8 + 2] = this.radii[i];
    }
    for (let k = 0; k < this.edgeCount; k++) {
      const a = store.nodes[this.edgePairs[k * 2]].id;
      const b = store.nodes[this.edgePairs[k * 2 + 1]].id;
      let s = 0;
      if (hasSel) s = a === sel || b === sel ? 1 : 3;
      else if (hov !== null && (a === hov || b === hov)) s = 1;
      this.edgeData[k * 10 + 9] = s;
    }
    this.dirtyState = false;
  }

  private render(): void {
    const gl = this.gl;
    if (this.dirtyGeometry) this.rebuildGeometry();
    if (this.dirtyPositions) this.updatePositions();
    if (this.dirtyState) this.updateState();
    gl.clearColor(0, 0, 0, 0);
    gl.clear(gl.COLOR_BUFFER_BIT);
    this.drawCalls = 0;
    const cam = store.camera;
    const view = [cam.x, cam.y, cam.zoom, this.dpr];
    const res = [this.width, this.height];
    if (this.nodeCount === 0) { this.drawLabels(); return; }

    gl.bindBuffer(gl.ARRAY_BUFFER, this.patchBuf);
    gl.bufferData(gl.ARRAY_BUFFER, this.patchData, gl.DYNAMIC_DRAW);
    gl.bindBuffer(gl.ARRAY_BUFFER, this.edgeBuf);
    gl.bufferData(gl.ARRAY_BUFFER, this.edgeData, gl.DYNAMIC_DRAW);
    gl.bindBuffer(gl.ARRAY_BUFFER, this.nodeBuf);
    gl.bufferData(gl.ARRAY_BUFFER, this.nodeData, gl.DYNAMIC_DRAW);

    if (this.patchCount > 0 && store.level !== "package") {
      gl.useProgram(this.patchProg);
      gl.uniform4fv(gl.getUniformLocation(this.patchProg, "u_view"), view);
      gl.uniform2fv(gl.getUniformLocation(this.patchProg, "u_res"), res);
      gl.bindVertexArray(this.patchVao);
      gl.drawArraysInstanced(gl.TRIANGLE_STRIP, 0, 4, this.patchCount);
      this.drawCalls++;
    }
    if (this.edgeCount > 0) {
      gl.useProgram(this.edgeProg);
      gl.uniform4fv(gl.getUniformLocation(this.edgeProg, "u_view"), view);
      gl.uniform2fv(gl.getUniformLocation(this.edgeProg, "u_res"), res);
      gl.uniform1f(gl.getUniformLocation(this.edgeProg, "u_time"), this.time);
      gl.bindVertexArray(this.edgeVao);
      gl.drawArraysInstanced(gl.TRIANGLE_STRIP, 0, 4, this.edgeCount);
      this.drawCalls++;
    }
    gl.useProgram(this.nodeProg);
    gl.uniform4fv(gl.getUniformLocation(this.nodeProg, "u_view"), view);
    gl.uniform2fv(gl.getUniformLocation(this.nodeProg, "u_res"), res);
    gl.bindVertexArray(this.nodeVao);
    gl.drawArraysInstanced(gl.TRIANGLE_STRIP, 0, 4, this.nodeCount);
    this.drawCalls++;
    gl.bindVertexArray(null);
    this.drawLabels();
  }

  // ---- labels ------------------------------------------------------------

  private drawLabels(): void {
    const ctx = this.labels;
    ctx.setTransform(this.dpr, 0, 0, this.dpr, 0, 0);
    ctx.clearRect(0, 0, this.width, this.height);
    this.labelsDrawn = 0;
    if (this.nodeCount === 0) return;
    const cam = store.camera;
    const cell = 22;
    const cols = Math.ceil(this.width / cell) + 1;
    const occupied = new Uint8Array(cols * (Math.ceil(this.height / cell) + 1));
    const pos = store.positions;
    const sel = store.selection;
    const hov = store.hover;
    // draw priority: selected, hover, neighbours, then big → small
    const order: number[] = [];
    for (let i = 0; i < this.nodeCount; i++) if (this.visible[i]) order.push(i);
    order.sort((a, b) => {
      const ia = store.nodes[a].id, ib = store.nodes[b].id;
      const pa = ia === sel ? 3 : ia === hov ? 2 : store.neighbourIds.has(ia) ? 1 : 0;
      const pb = ib === sel ? 3 : ib === hov ? 2 : store.neighbourIds.has(ib) ? 1 : 0;
      return pb - pa || this.radii[b] - this.radii[a];
    });
    ctx.textBaseline = "middle";
    let budget = 260;
    for (const i of order) {
      if (budget <= 0) break;
      const node = store.nodes[i];
      const r = Math.min(Math.max(this.radii[i] * cam.zoom, 2.5), 64);
      const important = node.id === sel || node.id === hov || store.neighbourIds.has(node.id);
      if (r < 5.5 && !important && (node.kind !== "package" || node.external)) continue;
      const sx = (pos[i * 2] - cam.x) * cam.zoom + this.width / 2;
      const sy = (pos[i * 2 + 1] - cam.y) * cam.zoom + this.height / 2;
      if (sx < -80 || sy < -20 || sx > this.width + 80 || sy > this.height + 20) continue;
      const isPkg = node.kind === "package" && !node.external;
      const size = isPkg ? 14 : node.id === sel ? 13 : node.external ? 10.5 : 11.5;
      ctx.font = isPkg ? `500 ${size}px Fraunces, Georgia, serif` : `${node.external ? "italic " : ""}${node.id === sel ? 500 : 400} ${size}px "IBM Plex Sans", system-ui, sans-serif`;
      const text = node.name;
      const w = ctx.measureText(text).width;
      const x = sx + r + 6;
      const y = sy;
      // occupancy test over the label's cells
      const c0 = Math.floor(x / cell), c1 = Math.floor((x + w) / cell), row = Math.floor(y / cell);
      let free = true;
      if (!important) {
        for (let c = c0; c <= c1 && free; c++) { const k = row * cols + c; if (k >= 0 && k < occupied.length && occupied[k]) free = false; }
      }
      if (!free) continue;
      for (let c = c0; c <= c1; c++) { const k = row * cols + c; if (k >= 0 && k < occupied.length) occupied[k] = 1; }
      const dim = sel !== null && !important;
      ctx.fillStyle = dim ? "rgba(217,228,218,0.28)" : node.id === sel ? "#F2B950" : isPkg ? "rgba(217,228,218,0.92)" : node.external ? "rgba(143,163,150,0.75)" : "rgba(217,228,218,0.82)";
      ctx.shadowColor = "rgba(10,16,13,0.9)";
      ctx.shadowBlur = 4;
      ctx.fillText(text, x, y);
      this.labelsDrawn++;
      budget--;
    }
    ctx.shadowBlur = 0;
  }

  // ---- picking -----------------------------------------------------------

  private grid = new Map<number, number[]>();
  private gridCell = 40;
  private gridDirty = true;

  private rebuildGrid(): void {
    this.grid.clear();
    const pos = store.positions;
    let maxR = 1;
    for (let i = 0; i < this.nodeCount; i++) if (this.radii[i] > maxR) maxR = this.radii[i];
    this.gridCell = Math.max(40, maxR * 2.5);
    for (let i = 0; i < this.nodeCount; i++) {
      if (!this.visible[i]) continue;
      const key = this.gridKey(pos[i * 2], pos[i * 2 + 1]);
      let bucket = this.grid.get(key);
      if (!bucket) { bucket = []; this.grid.set(key, bucket); }
      bucket.push(i);
    }
    this.gridDirty = false;
  }

  private gridKey(x: number, y: number): number {
    return (Math.floor(x / this.gridCell) + 32768) * 65536 + (Math.floor(y / this.gridCell) + 32768);
  }

  /** Node index under a screen point, or -1. */
  pick(sx: number, sy: number): number {
    if (this.nodeCount === 0) return -1;
    if (this.gridDirty) this.rebuildGrid();
    const cam = store.camera;
    const wx = (sx - this.width / 2) / cam.zoom + cam.x;
    const wy = (sy - this.height / 2) / cam.zoom + cam.y;
    const pos = store.positions;
    let best = -1, bestD = Infinity;
    const cx = Math.floor(wx / this.gridCell), cy = Math.floor(wy / this.gridCell);
    for (let dx = -1; dx <= 1; dx++) for (let dy = -1; dy <= 1; dy++) {
      const bucket = this.grid.get((cx + dx + 32768) * 65536 + (cy + dy + 32768));
      if (!bucket) continue;
      for (const i of bucket) {
        const rPx = Math.min(Math.max(this.radii[i] * cam.zoom, 2.5), 64) + 3;
        const d = Math.hypot((pos[i * 2] - wx) * cam.zoom, (pos[i * 2 + 1] - wy) * cam.zoom);
        if (d <= rPx && d < bestD) { best = i; bestD = d; }
      }
    }
    return best;
  }

  worldToScreen(x: number, y: number): [number, number] {
    const cam = store.camera;
    return [(x - cam.x) * cam.zoom + this.width / 2, (y - cam.y) * cam.zoom + this.height / 2];
  }

  screenToWorld(sx: number, sy: number): [number, number] {
    const cam = store.camera;
    return [(sx - this.width / 2) / cam.zoom + cam.x, (sy - this.height / 2) / cam.zoom + cam.y];
  }

  /** Bounding box of visible nodes in world units. */
  bounds(): { minX: number; minY: number; maxX: number; maxY: number } | null {
    if (this.dirtyGeometry) this.rebuildGeometry();
    const pos = store.positions;
    let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
    for (let i = 0; i < this.nodeCount; i++) {
      if (!this.visible[i]) continue;
      const x = pos[i * 2], y = pos[i * 2 + 1], r = this.radii[i];
      if (x - r < minX) minX = x - r; if (x + r > maxX) maxX = x + r;
      if (y - r < minY) minY = y - r; if (y + r > maxY) maxY = y + r;
    }
    return isFinite(minX) ? { minX, minY, maxX, maxY } : null;
  }

  fit(padding = 90): void {
    const b = this.bounds();
    if (!b) return;
    const w = Math.max(b.maxX - b.minX, 10), h = Math.max(b.maxY - b.minY, 10);
    const availW = this.width - (store.shelfOpen ? 330 : 40) - (store.selection !== null ? 370 : 40) - padding;
    const availH = this.height - 130 - padding;
    const zoom = Math.min(availW / w, availH / h, 6);
    // offset so the graph centres in the free space between the panels
    const freeCenterX = (store.shelfOpen ? 330 : 40) + availW / 2 + padding / 2;
    const freeCenterY = 52 + availH / 2 + padding / 2;
    store.camera.zoom = Math.max(zoom, 0.05);
    store.camera.x = (b.minX + b.maxX) / 2 - (freeCenterX - this.width / 2) / store.camera.zoom;
    store.camera.y = (b.minY + b.maxY) / 2 - (freeCenterY - this.height / 2) / store.camera.zoom;
    emit("camera");
  }

  stats(): FrameStats {
    const sorted = [...this.frameTimes].sort((a, b) => a - b);
    const p = (q: number) => (sorted.length ? sorted[Math.min(sorted.length - 1, Math.floor(q * sorted.length))] : 0);
    const idle = performance.now() - this.lastRenderAt > 500;
    const fps = idle ? 0 : this.frameTimes.length > 1 ? Math.min(60, 1000 / Math.max(p(0.5), 1000 / 60)) : 0;
    let nodesDrawn = 0;
    for (let i = 0; i < this.nodeCount; i++) if (this.visible[i]) nodesDrawn++;
    return { fps: Math.round(fps), frame_ms_p50: +p(0.5).toFixed(2), frame_ms_p95: +p(0.95).toFixed(2), draw_calls: this.drawCalls, nodes_drawn: nodesDrawn, edges_drawn: this.edgeCount, labels_drawn: this.labelsDrawn, renderer: this.rendererName };
  }

  /** Composite of the WebGL canvas and labels as a PNG data URL (no DOM panels). */
  snapshotDataUrl(): string {
    this.render();
    const out = document.createElement("canvas");
    out.width = this.canvas.width;
    out.height = this.canvas.height;
    const c = out.getContext("2d")!;
    c.fillStyle = "#17211c";
    c.fillRect(0, 0, out.width, out.height);
    c.drawImage(this.canvas, 0, 0);
    c.drawImage(this.labels.canvas, 0, 0);
    return out.toDataURL("image/png");
  }

  get lastFrameAt(): number {
    return this.lastFrame;
  }
}
