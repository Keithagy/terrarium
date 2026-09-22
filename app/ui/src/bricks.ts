// The brick model: the repository as a small town of bricks, drawn with three.js.
// Districts (packages) are baseplates set in canal water, buildings (files) are
// stacks of bricks (symbols) coloured by language, and cross-language flows are
// amber bridges. Everything is instanced so a few thousand bricks stay a handful of
// draw calls, and the scene renders only when something changes.

import * as THREE from "three";
import { LANG_COLORS, type BuildModel, type Lang } from "./types";

export type View = "iso" | "front" | "top";
export interface Hit { building: number; brick: number }

const BRICK_H = 1.2;
const PLATE_H = 0.4;
const STUD_R = 0.3;
const STUD_H = 0.17;
const GAP = 0.04; // seam between neighbouring bricks
const BRIDGE_R = 0.2;
const DROP = 7; // how far a new piece falls from, in studs
const DROP_MS = 350;
const EASE_MS = 450;

const AMBER = new THREE.Color().setRGB(0xf2 / 255, 0xb9 / 255, 0x50 / 255, THREE.SRGBColorSpace);
const DIM = new THREE.Color().setRGB(0.23, 0.29, 0.26, THREE.SRGBColorSpace);
const SAND = new THREE.Color().setRGB(0.80, 0.75, 0.62, THREE.SRGBColorSpace);
const WATER = new THREE.Color().setRGB(0.21, 0.40, 0.42, THREE.SRGBColorSpace);
const WHITE = new THREE.Color(1, 1, 1);

const VIEWS: Record<View, { az: number; el: number }> = {
  iso: { az: Math.PI / 4, el: (35 * Math.PI) / 180 },
  front: { az: 0, el: (14 * Math.PI) / 180 },
  top: { az: 0, el: (89.5 * Math.PI) / 180 },
};

function langColor(l: Lang): THREE.Color {
  const [r, g, b] = LANG_COLORS[l] ?? LANG_COLORS.other;
  return new THREE.Color().setRGB(r, g, b, THREE.SRGBColorSpace);
}

const reducedMotion = () => matchMedia("(prefers-reduced-motion: reduce)").matches;
const easeOut = (t: number) => 1 - Math.pow(1 - t, 3);

interface CameraState { az: number; el: number; zoom: number; target: THREE.Vector3 }

export class BrickScene {
  readonly rendererName: string;
  private host: HTMLElement;
  private canvas: HTMLCanvasElement;
  private renderer: THREE.WebGLRenderer;
  private scene = new THREE.Scene();
  private camera = new THREE.OrthographicCamera(-1, 1, 1, -1, 0.1, 2000);
  private sun: THREE.DirectionalLight;
  private root = new THREE.Group();
  private raycaster = new THREE.Raycaster();

  private model: BuildModel | null = null;
  private stepCount = 0;
  private _step = -1;
  private _spinning = false;

  // meshes
  private bricks: THREE.InstancedMesh | null = null;
  private studs: THREE.InstancedMesh | null = null;
  private lamps: THREE.InstancedMesh | null = null;
  private bridgeMesh: THREE.InstancedMesh | null = null;
  private outlines = new THREE.Group();
  private selectOutline = new THREE.Group();

  // per-instance bookkeeping
  private brickBase: THREE.Color[] = [];
  private studOwner: Int32Array = new Int32Array(0); // building index, or -1 for district studs
  private studPos: Float32Array = new Float32Array(0);
  private lampOwner: number[] = [];
  private bridgeSeg: { bridge: number; a: THREE.Vector3; b: THREE.Vector3 }[] = [];
  private topBrick: number[] = []; // building -> index of its top brick
  private drop: Float32Array = new Float32Array(0); // building -> current y offset
  private dropStart = 0;
  private dropping: number[] = [];

  private highlightSet: Set<number> | null = null;
  private selected: Hit | null = null;
  private hovered: Hit | null = null;

  // camera
  private cam: CameraState = { az: VIEWS.iso.az, el: VIEWS.iso.el, zoom: 10, target: new THREE.Vector3() };
  private camFrom: CameraState | null = null;
  private camTo: CameraState | null = null;
  private camStart = 0;

  // rendering
  private needsRender = true;
  private rafId = 0;
  private lastFrame = 0;
  private frameTimes: number[] = [];
  private frameStamps: number[] = [];

  private pickCb: ((hit: Hit | null, ev: MouseEvent) => void) | null = null;
  private hoverCb: ((hit: Hit | null) => void) | null = null;

  constructor(host: HTMLElement) {
    this.host = host;
    this.canvas = document.createElement("canvas");
    this.canvas.dataset.testid = "canvas";
    this.canvas.style.cssText = "position:absolute;inset:0;width:100%;height:100%;display:block;touch-action:none;";
    host.appendChild(this.canvas);
    this.renderer = new THREE.WebGLRenderer({ canvas: this.canvas, antialias: true, alpha: true, preserveDrawingBuffer: false });
    this.renderer.setClearColor(0x000000, 0);
    this.renderer.shadowMap.enabled = true;
    this.renderer.shadowMap.type = THREE.PCFSoftShadowMap;
    this.renderer.outputColorSpace = THREE.SRGBColorSpace;
    const gl = this.renderer.getContext();
    const dbg = gl.getExtension("WEBGL_debug_renderer_info");
    this.rendererName = dbg ? String(gl.getParameter(dbg.UNMASKED_RENDERER_WEBGL)) : String(gl.getParameter(gl.RENDERER));

    this.scene.add(new THREE.HemisphereLight(0xe8f0e6, 0x2a3a31, 1.25));
    this.sun = new THREE.DirectionalLight(0xfff2dc, 2.1);
    this.sun.castShadow = true;
    this.sun.shadow.mapSize.set(2048, 2048);
    this.sun.shadow.bias = -0.0006;
    this.sun.shadow.normalBias = 0.02;
    this.scene.add(this.sun, this.sun.target);
    this.scene.add(this.root);
    this.root.add(this.outlines, this.selectOutline);

    new ResizeObserver(() => this.resize()).observe(host);
    this.resize();
    this.bindInput();
  }

  get step(): number { return this._step; }
  get spinning(): boolean { return this._spinning; }

  // ---- model ------------------------------------------------------------------

  setModel(m: BuildModel, stepCount: number): void {
    this.clear();
    this.model = m;
    this.stepCount = stepCount;
    const [sw, sd] = m.studs;
    const ox = -sw / 2, oz = -sd / 2;
    const at = (x: number, z: number) => new THREE.Vector3(x + ox, 0, z + oz);

    const plastic = new THREE.MeshStandardMaterial({ color: 0xffffff, roughness: 0.42, metalness: 0 });

    // Canal water under everything.
    const water = new THREE.Mesh(new THREE.BoxGeometry(sw, PLATE_H, sd), new THREE.MeshStandardMaterial({ color: WATER, roughness: 0.25, metalness: 0.05 }));
    water.position.set(0, -PLATE_H / 2, 0);
    water.receiveShadow = true;
    this.root.add(water);

    // District baseplates.
    const unit = new THREE.BoxGeometry(1, 1, 1);
    const plates = new THREE.InstancedMesh(unit, plastic, Math.max(1, m.districts.length));
    plates.count = m.districts.length;
    const mtx = new THREE.Matrix4();
    const q = new THREE.Quaternion();
    m.districts.forEach((d, i) => {
      const c = at(d.x + d.w / 2, d.z + d.d / 2);
      mtx.compose(new THREE.Vector3(c.x, PLATE_H / 2, c.z), q, new THREE.Vector3(d.w - GAP * 2, PLATE_H, d.d - GAP * 2));
      plates.setMatrixAt(i, mtx);
      plates.setColorAt(i, langColor(d.lang).lerp(SAND, 0.68).multiplyScalar(0.92));
    });
    plates.receiveShadow = true;
    this.root.add(plates);

    // Bricks: one instance per brick.
    const nb = m.bricks.length;
    this.bricks = new THREE.InstancedMesh(unit, plastic, Math.max(1, nb));
    this.bricks.count = nb;
    this.bricks.castShadow = true;
    this.bricks.receiveShadow = true;
    this.topBrick = m.buildings.map(() => -1);
    this.brickBase = m.bricks.map((br) => {
      const b = m.buildings[br.building];
      if (this.topBrick[br.building] < 0 || m.bricks[this.topBrick[br.building]].layer < br.layer) this.topBrick[br.building] = m.bricks.indexOf(br);
      const c = langColor(b.lang);
      const hsl = { h: 0, s: 0, l: 0 };
      c.getHSL(hsl);
      c.setHSL(hsl.h, hsl.s * 0.92, Math.min(0.85, hsl.l + (br.layer % 2 === 0 ? 0.03 : -0.04)));
      return c;
    });
    this.root.add(this.bricks);

    // Studs: on the top brick of every building and on every uncovered plate cell.
    const covered = new Set<string>();
    for (const b of m.buildings) for (let x = b.x; x < b.x + b.w; x++) for (let z = b.z; z < b.z + b.d; z++) covered.add(`${x},${z}`);
    const studOwner: number[] = [];
    const studPos: number[] = [];
    m.buildings.forEach((b, bi) => {
      for (let x = b.x; x < b.x + b.w; x++) for (let z = b.z; z < b.z + b.d; z++) {
        studOwner.push(bi);
        studPos.push(x + 0.5 + ox, PLATE_H + b.layers * BRICK_H + STUD_H / 2, z + 0.5 + oz);
      }
    });
    for (const d of m.districts) for (let x = d.x; x < d.x + d.w; x++) for (let z = d.z; z < d.z + d.d; z++) {
      if (covered.has(`${x},${z}`)) continue;
      studOwner.push(-1);
      studPos.push(x + 0.5 + ox, PLATE_H + STUD_H / 2, z + 0.5 + oz);
    }
    this.studOwner = Int32Array.from(studOwner);
    this.studPos = Float32Array.from(studPos);
    const studGeo = new THREE.CylinderGeometry(STUD_R, STUD_R, STUD_H, 14);
    this.studs = new THREE.InstancedMesh(studGeo, plastic, Math.max(1, studOwner.length));
    this.studs.count = studOwner.length;
    this.studs.castShadow = true;
    this.root.add(this.studs);

    // Lamps: a round amber brick on landmark buildings.
    this.lampOwner = m.buildings.flatMap((b, i) => (b.lamp ? [i] : []));
    const lampMat = new THREE.MeshStandardMaterial({ color: AMBER, emissive: AMBER, emissiveIntensity: 0.55, roughness: 0.35 });
    this.lamps = new THREE.InstancedMesh(new THREE.CylinderGeometry(0.42, 0.42, BRICK_H, 20), lampMat, Math.max(1, this.lampOwner.length));
    this.lamps.count = this.lampOwner.length;
    this.lamps.castShadow = true;
    this.root.add(this.lamps);

    // Bridges: arcs made of short instanced tube segments.
    this.bridgeSeg = [];
    m.bridges.forEach((br, i) => {
      const pts = this.bridgeArc(br.from, br.to, ox, oz);
      for (let k = 0; k + 1 < pts.length; k++) this.bridgeSeg.push({ bridge: i, a: pts[k], b: pts[k + 1] });
    });
    const bridgeMat = new THREE.MeshStandardMaterial({ color: AMBER, emissive: AMBER, emissiveIntensity: 0.35, roughness: 0.4 });
    const segGeo = new THREE.CylinderGeometry(BRIDGE_R, BRIDGE_R, 1, 8, 1, true);
    this.bridgeMesh = new THREE.InstancedMesh(segGeo, bridgeMat, Math.max(1, this.bridgeSeg.length));
    this.bridgeMesh.count = this.bridgeSeg.length;
    this.bridgeMesh.castShadow = true;
    this.root.add(this.bridgeMesh);

    // Light and shadow sized to the plate.
    const span = Math.max(sw, sd);
    this.sun.position.set(-span * 0.6, span * 1.1, span * 0.8);
    this.sun.target.position.set(0, 0, 0);
    const sc = this.sun.shadow.camera;
    sc.left = -span; sc.right = span; sc.top = span; sc.bottom = -span; sc.near = 0.1; sc.far = span * 4;
    sc.updateProjectionMatrix();

    this.drop = new Float32Array(m.buildings.length);
    this._step = Math.max(-1, stepCount - 1);
    this.dropping = [];
    this.writeAll();
    this.fitNow();
    this.requestRender();
  }

  private bridgeArc(from: number, to: number, ox: number, oz: number): THREE.Vector3[] {
    const m = this.model!;
    const bf = m.bricks[from], bt = m.bricks[to];
    const A = m.buildings[bf.building], B = m.buildings[bt.building];
    const ca = new THREE.Vector2(A.x + A.w / 2, A.z + A.d / 2);
    const cb = new THREE.Vector2(B.x + B.w / 2, B.z + B.d / 2);
    // Leave each building through the wall facing the other one, at the brick's height.
    const exit = (c: THREE.Vector2, w: number, d: number, toward: THREE.Vector2) => {
      const dir = toward.clone().sub(c);
      if (dir.lengthSq() < 1e-6) return c.clone();
      const t = Math.min(dir.x !== 0 ? w / 2 / Math.abs(dir.x) : Infinity, dir.y !== 0 ? d / 2 / Math.abs(dir.y) : Infinity);
      return c.clone().add(dir.multiplyScalar(t * 1.02));
    };
    const pa = exit(ca, A.w, A.d, cb), pb = exit(cb, B.w, B.d, ca);
    const ya = PLATE_H + (bf.layer + 0.5) * BRICK_H, yb = PLATE_H + (bt.layer + 0.5) * BRICK_H;
    const a = new THREE.Vector3(pa.x + ox, ya, pa.y + oz);
    const b = new THREE.Vector3(pb.x + ox, yb, pb.y + oz);
    const dist = Math.hypot(a.x - b.x, a.z - b.z);
    const tallest = Math.max(A.layers, B.layers) * BRICK_H + PLATE_H;
    const mid = a.clone().add(b).multiplyScalar(0.5);
    mid.y = Math.max(ya, yb, tallest * 0.6) + 1.2 + dist * 0.32;
    const curve = new THREE.QuadraticBezierCurve3(a, mid, b);
    const n = Math.max(8, Math.ceil(curve.getLength() / 0.7));
    return curve.getPoints(n);
  }

  private clear(): void {
    for (const child of [...this.root.children]) {
      if (child === this.outlines || child === this.selectOutline) continue;
      this.root.remove(child);
      const mesh = child as THREE.Mesh;
      mesh.geometry?.dispose();
      const mat = mesh.material as THREE.Material | THREE.Material[] | undefined;
      if (Array.isArray(mat)) mat.forEach((x) => x.dispose()); else mat?.dispose();
    }
    this.clearGroup(this.outlines);
    this.clearGroup(this.selectOutline);
    this.bricks = this.studs = this.lamps = this.bridgeMesh = null;
    this.highlightSet = null;
    this.selected = null;
    this.hovered = null;
  }

  private clearGroup(g: THREE.Group): void {
    for (const c of [...g.children]) {
      g.remove(c);
      (c as THREE.LineSegments).geometry?.dispose();
    }
  }

  // ---- step, highlight, selection -------------------------------------------------

  setStep(step: number, opts: { animate?: boolean } = {}): void {
    if (!this.model) return;
    const s = Math.max(-1, Math.min(step, this.stepCount - 1));
    const prev = this._step;
    this._step = s;
    this.drop.fill(0);
    this.dropping = [];
    if (opts.animate && !reducedMotion() && s > prev) {
      this.dropping = this.model.buildings.flatMap((b, i) => (b.step === s ? [i] : []));
      for (const i of this.dropping) this.drop[i] = DROP;
      this.dropStart = performance.now();
    }
    this.writeAll();
    this.requestRender();
  }

  highlight(buildings: Set<number> | null): void {
    this.highlightSet = buildings && buildings.size ? new Set(buildings) : null;
    this.writeColors();
    this.requestRender();
  }

  select(sel: Hit | null): void {
    this.selected = sel;
    this.writeSelection();
    this.writeColors();
    this.requestRender();
  }

  private visibleBuilding(i: number): boolean {
    return this.model!.buildings[i].step <= this._step;
  }

  /** Matrices and colours for everything; cheap enough to redo on every step change. */
  private writeAll(): void {
    this.writeMatrices(null);
    this.writeColors();
    this.writeStepOutlines();
    this.writeSelection();
  }

  private writeMatrices(only: Set<number> | null): void {
    const m = this.model;
    if (!m || !this.bricks || !this.studs || !this.lamps || !this.bridgeMesh) return;
    const [sw, sd] = m.studs;
    const ox = -sw / 2, oz = -sd / 2;
    const mtx = new THREE.Matrix4();
    const q = new THREE.Quaternion();
    const zero = new THREE.Matrix4().makeScale(0, 0, 0);
    m.bricks.forEach((br, i) => {
      if (only && !only.has(br.building)) return;
      const b = m.buildings[br.building];
      if (!this.visibleBuilding(br.building)) { this.bricks!.setMatrixAt(i, zero); return; }
      const y = PLATE_H + (br.layer + 0.5) * BRICK_H + this.drop[br.building];
      mtx.compose(new THREE.Vector3(b.x + b.w / 2 + ox, y, b.z + b.d / 2 + oz), q, new THREE.Vector3(b.w - GAP * 2, BRICK_H - GAP / 2, b.d - GAP * 2));
      this.bricks!.setMatrixAt(i, mtx);
    });
    this.bricks.instanceMatrix.needsUpdate = true;
    this.bricks.computeBoundingSphere();
    for (let i = 0; i < this.studOwner.length; i++) {
      const owner = this.studOwner[i];
      if (only && (owner < 0 || !only.has(owner))) continue;
      if (owner >= 0 && !this.visibleBuilding(owner)) { this.studs.setMatrixAt(i, zero); continue; }
      const dy = owner >= 0 ? this.drop[owner] : 0;
      mtx.makeTranslation(this.studPos[i * 3], this.studPos[i * 3 + 1] + dy, this.studPos[i * 3 + 2]);
      this.studs.setMatrixAt(i, mtx);
    }
    this.studs.instanceMatrix.needsUpdate = true;
    this.studs.computeBoundingSphere();
    this.lampOwner.forEach((bi, i) => {
      if (only && !only.has(bi)) return;
      const b = m.buildings[bi];
      if (!this.visibleBuilding(bi)) { this.lamps!.setMatrixAt(i, zero); return; }
      // Sits on the front-left stud of the roof, like a lamp post on a corner.
      const y = PLATE_H + b.layers * BRICK_H + BRICK_H / 2 + this.drop[bi];
      mtx.makeTranslation(b.x + 0.5 + ox, y, b.z + b.d - 0.5 + oz);
      this.lamps!.setMatrixAt(i, mtx);
    });
    this.lamps.instanceMatrix.needsUpdate = true;
    this.lamps.computeBoundingSphere();
    // A bridge appears once both ends stand still.
    const up = new THREE.Vector3(0, 1, 0);
    this.bridgeSeg.forEach((seg, i) => {
      const br = m.bridges[seg.bridge];
      const ends = [m.bricks[br.from].building, m.bricks[br.to].building];
      if (only && !ends.some((e) => only.has(e))) return;
      const moving = ends.some((e) => this.drop[e] > 0.001);
      if (br.step > this._step || moving) { this.bridgeMesh!.setMatrixAt(i, zero); return; }
      const dir = seg.b.clone().sub(seg.a);
      const len = dir.length();
      q.setFromUnitVectors(up, dir.normalize());
      mtx.compose(seg.a.clone().add(seg.b).multiplyScalar(0.5), q, new THREE.Vector3(1, len * 1.04, 1));
      this.bridgeMesh!.setMatrixAt(i, mtx);
    });
    q.identity();
    this.bridgeMesh.instanceMatrix.needsUpdate = true;
    this.bridgeMesh.computeBoundingSphere();
    for (const g of [this.outlines, this.selectOutline]) for (const c of g.children) {
      const bi = c.userData.building as number | undefined;
      if (bi !== undefined) c.position.y = (c.userData.baseY as number) + this.drop[bi];
    }
  }

  private writeColors(): void {
    const m = this.model;
    if (!m || !this.bricks || !this.studs) return;
    const c = new THREE.Color();
    const hl = this.highlightSet;
    const color = (bi: number, base: THREE.Color, brick: number | null) => {
      c.copy(base);
      // New in this step: a little lighter (amber would turn blue bricks grey); the outline says the rest.
      if (m.buildings[bi].step === this._step && this._step < this.stepCount - 1) c.lerp(WHITE, 0.16);
      if (hl && !hl.has(bi)) c.lerp(DIM, 0.72);
      if (brick !== null && this.hovered && this.hovered.brick === brick) c.lerp(WHITE, 0.22);
      if (brick !== null && this.selected && this.selected.brick === brick) c.lerp(AMBER, 0.45);
      return c;
    };
    m.bricks.forEach((br, i) => this.bricks!.setColorAt(i, color(br.building, this.brickBase[i], i)));
    if (this.bricks.instanceColor) this.bricks.instanceColor.needsUpdate = true;
    const plate = new THREE.Color();
    for (let i = 0; i < this.studOwner.length; i++) {
      const owner = this.studOwner[i];
      if (owner >= 0) {
        const top = this.topBrick[owner];
        this.studs.setColorAt(i, color(owner, this.brickBase[top], null));
      } else {
        // Plate studs take their plate's colour.
        const x = this.studPos[i * 3] + m.studs[0] / 2, z = this.studPos[i * 3 + 2] + m.studs[1] / 2;
        const d = m.districts.find((dd) => x >= dd.x && x < dd.x + dd.w && z >= dd.z && z < dd.z + dd.d);
        plate.copy(d ? langColor(d.lang).lerp(SAND, 0.68).multiplyScalar(0.92) : SAND);
        if (hl) plate.lerp(DIM, 0.35);
        this.studs.setColorAt(i, plate);
      }
    }
    if (this.studs.instanceColor) this.studs.instanceColor.needsUpdate = true;
    if (this.bridgeMesh) {
      this.bridgeSeg.forEach((seg, i) => {
        const br = m.bridges[seg.bridge];
        const lit = !hl || (hl.has(m.bricks[br.from].building) && hl.has(m.bricks[br.to].building));
        this.bridgeMesh!.setColorAt(i, lit ? new THREE.Color(1, 1, 1) : new THREE.Color(0.35, 0.35, 0.35));
      });
      if (this.bridgeMesh.instanceColor) this.bridgeMesh.instanceColor.needsUpdate = true;
    }
  }

  private boxOutline(bi: number, color: THREE.Color, opacity: number, grow: number, layer?: number): THREE.LineSegments {
    const m = this.model!;
    const b = m.buildings[bi];
    const [sw, sd] = m.studs;
    const h = layer === undefined ? b.layers * BRICK_H : BRICK_H;
    const y0 = PLATE_H + (layer ?? 0) * BRICK_H;
    const geo = new THREE.EdgesGeometry(new THREE.BoxGeometry(b.w + grow, h + grow, b.d + grow));
    const line = new THREE.LineSegments(geo, new THREE.LineBasicMaterial({ color, transparent: true, opacity, depthTest: true }));
    line.position.set(b.x + b.w / 2 - sw / 2, y0 + h / 2, b.z + b.d / 2 - sd / 2);
    line.userData = { building: bi, baseY: y0 + h / 2 };
    return line;
  }

  private writeStepOutlines(): void {
    this.clearGroup(this.outlines);
    const m = this.model;
    if (!m || this._step >= this.stepCount - 1) return; // the finished model has no "new" pieces
    m.buildings.forEach((b, i) => {
      if (b.step === this._step) this.outlines.add(this.boxOutline(i, AMBER, 0.9, 0.08));
    });
  }

  private writeSelection(): void {
    this.clearGroup(this.selectOutline);
    const m = this.model;
    const s = this.selected;
    if (!m || !s || s.building < 0 || s.building >= m.buildings.length || !this.visibleBuilding(s.building)) return;
    this.selectOutline.add(this.boxOutline(s.building, new THREE.Color(1, 1, 1), 0.35, 0.12));
    const br = m.bricks[s.brick];
    if (br && br.building === s.building) this.selectOutline.add(this.boxOutline(s.building, AMBER, 1, 0.16, br.layer));
  }

  // ---- camera ----------------------------------------------------------------------

  setView(v: View): void {
    const to = { ...this.cam, target: this.cam.target.clone(), az: VIEWS[v].az, el: VIEWS[v].el };
    // Take the short way round from wherever a spin left the azimuth.
    const tau = Math.PI * 2;
    to.az = this.cam.az + ((((to.az - this.cam.az) % tau) + tau + Math.PI) % tau) - Math.PI;
    this._spinning = false;
    this.easeTo(to);
  }

  setSpin(on: boolean): void {
    this._spinning = on;
    this.lastFrame = performance.now();
    this.requestRender();
  }

  fit(): void {
    this.easeTo(this.fitState());
  }

  focusDistrict(index: number | null): void {
    const m = this.model;
    if (!m || index === null || !m.districts[index]) { this.fit(); return; }
    const d = m.districts[index];
    const tallest = Math.max(1, ...m.buildings.filter((b) => b.district === index).map((b) => b.layers)) * BRICK_H;
    const target = new THREE.Vector3(d.x + d.w / 2 - m.studs[0] / 2, tallest / 3, d.z + d.d / 2 - m.studs[1] / 2);
    const r = Math.hypot(d.w, d.d, tallest) / 2;
    this.easeTo({ ...this.cam, target, zoom: this.zoomFor(r * 1.35) });
  }

  private zoomFor(radius: number): number {
    const w = this.host.clientWidth || 800, h = this.host.clientHeight || 600;
    return Math.min(w, h) / (2 * Math.max(radius, 1));
  }

  private fitState(): CameraState {
    const m = this.model;
    if (!m) return { ...this.cam };
    const tallest = Math.max(1, ...m.buildings.map((b) => b.layers)) * BRICK_H;
    const r = Math.hypot(m.studs[0], m.studs[1]) / 2;
    return { ...this.cam, target: new THREE.Vector3(0, tallest / 4, 0), zoom: this.zoomFor(Math.max(r * 0.78, tallest)) };
  }

  private fitNow(): void {
    const s = this.fitState();
    this.cam = { ...s, az: VIEWS.iso.az, el: VIEWS.iso.el };
    this.camFrom = this.camTo = null;
    this.applyCamera();
  }

  private easeTo(to: CameraState): void {
    if (reducedMotion()) {
      this.cam = { ...to, target: to.target.clone() };
      this.camFrom = this.camTo = null;
    } else {
      this.camFrom = { ...this.cam, target: this.cam.target.clone() };
      this.camTo = { ...to, target: to.target.clone() };
      this.camStart = performance.now();
    }
    this.requestRender();
  }

  private applyCamera(): void {
    const { az, el, zoom, target } = this.cam;
    const dist = 400;
    this.camera.position.set(
      target.x + dist * Math.cos(el) * Math.sin(az),
      target.y + dist * Math.sin(el),
      target.z + dist * Math.cos(el) * Math.cos(az),
    );
    this.camera.up.set(0, 1, 0);
    if (el > 1.5) this.camera.up.set(-Math.sin(az), 0, -Math.cos(az)); // looking straight down: keep "north" up
    this.camera.lookAt(target);
    this.camera.zoom = zoom;
    this.camera.far = dist * 3;
    this.camera.updateProjectionMatrix();
  }

  private resize(): void {
    const w = Math.max(1, this.host.clientWidth), h = Math.max(1, this.host.clientHeight);
    this.renderer.setPixelRatio(Math.min(window.devicePixelRatio || 1, 2));
    this.renderer.setSize(w, h, false);
    this.camera.left = -w / 2; this.camera.right = w / 2; this.camera.top = h / 2; this.camera.bottom = -h / 2;
    this.camera.updateProjectionMatrix();
    this.requestRender();
  }

  // ---- input -----------------------------------------------------------------------

  onPick(cb: (hit: Hit | null, ev: MouseEvent) => void): void { this.pickCb = cb; }
  onHover(cb: (hit: Hit | null) => void): void { this.hoverCb = cb; }

  private bindInput(): void {
    const c = this.canvas;
    let drag: { x: number; y: number; button: number; moved: boolean; cam: CameraState } | null = null;
    c.addEventListener("contextmenu", (e) => e.preventDefault());
    c.addEventListener("pointerdown", (e) => {
      c.setPointerCapture(e.pointerId);
      this.camFrom = this.camTo = null;
      drag = { x: e.clientX, y: e.clientY, button: e.button, moved: false, cam: { ...this.cam, target: this.cam.target.clone() } };
    });
    c.addEventListener("pointermove", (e) => {
      if (drag) {
        const dx = e.clientX - drag.x, dy = e.clientY - drag.y;
        if (Math.abs(dx) + Math.abs(dy) > 3) drag.moved = true;
        if (!drag.moved) return;
        if (drag.button === 2 || e.shiftKey) {
          this.cam.target.copy(drag.cam.target).add(this.screenDelta(-dx, dy));
        } else {
          this._spinning = false;
          this.cam.az = drag.cam.az - dx * 0.008;
          this.cam.el = Math.min(VIEWS.top.el, Math.max(0.12, drag.cam.el + dy * 0.006));
        }
        this.requestRender();
        return;
      }
      this.hoverAt(e.clientX, e.clientY);
    });
    c.addEventListener("pointerup", (e) => {
      const d = drag;
      drag = null;
      if (d && !d.moved) this.pickCb?.(this.pickAt(e.clientX, e.clientY), e);
    });
    c.addEventListener("pointerleave", () => { if (!drag) this.setHovered(null); });
    c.addEventListener("wheel", (e) => {
      e.preventDefault();
      this.camFrom = this.camTo = null;
      if (e.ctrlKey || e.metaKey) {
        // Zoom about the cursor: the point under it stays put.
        const before = this.worldAtCursor(e.clientX, e.clientY);
        const factor = Math.exp(-e.deltaY * (e.ctrlKey ? 0.012 : 0.0025));
        this.cam.zoom = Math.min(Math.max(this.cam.zoom * factor, 1), 400);
        this.applyCamera();
        const after = this.worldAtCursor(e.clientX, e.clientY);
        this.cam.target.add(before.sub(after));
      } else {
        this.cam.target.add(this.screenDelta(e.deltaX, -e.deltaY));
      }
      this.requestRender();
    }, { passive: false });
  }

  /** A screen-space move in CSS pixels, as a world-space move on the view plane. */
  private screenDelta(dx: number, dy: number): THREE.Vector3 {
    this.applyCamera();
    const right = new THREE.Vector3().setFromMatrixColumn(this.camera.matrixWorld, 0);
    const up = new THREE.Vector3().setFromMatrixColumn(this.camera.matrixWorld, 1);
    const k = 1 / this.cam.zoom;
    return right.multiplyScalar(dx * k).add(up.multiplyScalar(dy * k));
  }

  private worldAtCursor(x: number, y: number): THREE.Vector3 {
    const r = this.canvas.getBoundingClientRect();
    const ndc = new THREE.Vector3(((x - r.left) / r.width) * 2 - 1, -((y - r.top) / r.height) * 2 + 1, 0);
    return ndc.unproject(this.camera);
  }

  private pickAt(x: number, y: number): Hit | null {
    const m = this.model;
    if (!m || !this.bricks) return null;
    const r = this.canvas.getBoundingClientRect();
    const ndc = new THREE.Vector2(((x - r.left) / r.width) * 2 - 1, -((y - r.top) / r.height) * 2 + 1);
    this.applyCamera();
    this.raycaster.setFromCamera(ndc, this.camera);
    const targets = [this.bricks, this.studs, this.lamps].filter((t): t is THREE.InstancedMesh => !!t);
    const hit = this.raycaster.intersectObjects(targets, false)[0];
    if (!hit || hit.instanceId === undefined) return null;
    if (hit.object === this.bricks) return { building: m.bricks[hit.instanceId].building, brick: hit.instanceId };
    const owner = hit.object === this.studs ? this.studOwner[hit.instanceId] : this.lampOwner[hit.instanceId];
    if (owner === undefined || owner < 0) return null;
    return { building: owner, brick: this.topBrick[owner] };
  }

  private hoverPending: { x: number; y: number } | null = null;
  private hoverAt(x: number, y: number): void {
    const first = this.hoverPending === null;
    this.hoverPending = { x, y };
    if (!first) return;
    requestAnimationFrame(() => {
      const p = this.hoverPending!;
      this.hoverPending = null;
      this.setHovered(this.pickAt(p.x, p.y));
    });
  }

  private setHovered(h: Hit | null): void {
    if (this.hovered?.brick === h?.brick && this.hovered?.building === h?.building) return;
    this.hovered = h;
    this.canvas.style.cursor = h ? "pointer" : "";
    this.writeColors();
    this.requestRender();
    this.hoverCb?.(h);
  }

  // ---- render loop -----------------------------------------------------------------

  private requestRender(): void {
    this.needsRender = true;
    if (!this.rafId) this.rafId = requestAnimationFrame((t) => this.frame(t));
  }

  private frame(now: number): void {
    this.rafId = 0;
    let busy = false;
    if (this.camFrom && this.camTo) {
      const t = Math.min(1, (now - this.camStart) / EASE_MS);
      const k = easeOut(t);
      const a = this.camFrom, b = this.camTo;
      this.cam = { az: a.az + (b.az - a.az) * k, el: a.el + (b.el - a.el) * k, zoom: a.zoom + (b.zoom - a.zoom) * k, target: a.target.clone().lerp(b.target, k) };
      if (t >= 1) this.camFrom = this.camTo = null; else busy = true;
    }
    if (this._spinning) {
      const dt = Math.min(0.1, (now - (this.lastFrame || now)) / 1000);
      this.cam.az += dt * 0.35;
      busy = true;
    }
    if (this.dropping.length) {
      const t = Math.min(1, (now - this.dropStart) / DROP_MS);
      // Fall and settle: ease-in drop with a small bounce at the end.
      const k = t < 0.8 ? Math.pow(t / 0.8, 2) : 1 - Math.sin(((t - 0.8) / 0.2) * Math.PI) * 0.06;
      for (const i of this.dropping) this.drop[i] = DROP * (1 - k);
      if (t >= 1) {
        for (const i of this.dropping) this.drop[i] = 0;
        this.dropping = [];
        // everything, so bridges that waited for both ends to land appear
        this.writeMatrices(null);
      } else {
        this.writeMatrices(new Set(this.dropping));
        busy = true;
      }
    }
    this.lastFrame = now;
    if (this.needsRender || busy) this.renderNow();
    this.needsRender = false;
    if (busy) this.rafId = requestAnimationFrame((t) => this.frame(t));
  }

  private renderNow(): void {
    const t0 = performance.now();
    this.applyCamera();
    this.renderer.render(this.scene, this.camera);
    const dt = performance.now() - t0;
    this.frameTimes.push(dt);
    if (this.frameTimes.length > 120) this.frameTimes.shift();
    this.frameStamps.push(t0);
    while (this.frameStamps.length && t0 - this.frameStamps[0] > 1000) this.frameStamps.shift();
  }

  stats(): { fps: number; frame_ms_p50: number; frame_ms_p95: number; draw_calls: number; pieces_drawn: number; renderer: string } {
    const sorted = [...this.frameTimes].sort((a, b) => a - b);
    const q = (p: number) => (sorted.length ? sorted[Math.min(sorted.length - 1, Math.floor(p * sorted.length))] : 0);
    const now = performance.now();
    const fps = this.frameStamps.filter((s) => now - s <= 1000).length;
    const pieces = this.model ? this.model.bricks.filter((b) => this.visibleBuilding(b.building)).length : 0;
    return {
      fps,
      frame_ms_p50: Math.round(q(0.5) * 100) / 100,
      frame_ms_p95: Math.round(q(0.95) * 100) / 100,
      draw_calls: this.renderer.info.render.calls,
      pieces_drawn: pieces,
      renderer: this.rendererName,
    };
  }

  snapshotDataUrl(): string {
    this.renderNow();
    return this.canvas.toDataURL("image/png");
  }
}
