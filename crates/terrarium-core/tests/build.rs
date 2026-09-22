use std::collections::HashSet;
use std::path::PathBuf;
use terrarium_core::{ScanOptions, build, scan};

fn fixture() -> terrarium_core::Graph {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/polyglot");
    scan(&root, &ScanOptions::default()).unwrap()
}

fn step_of(d: &build::Design, path: &str) -> usize {
    d.steps.iter().position(|s| s.files.iter().any(|f| f == path)).unwrap_or_else(|| panic!("{path} is not in any step"))
}

#[test]
fn engine_design_holds_together() {
    let g = fixture();
    let d = build::engine_design(&g);
    let c = build::check(&g, &d);
    assert!(c.ok, "weak joints in the engine's own design: {:#?}", c.weak);
    assert_eq!(c.files, 12);
    assert_eq!(c.steps as usize, d.steps.len());
    assert!(c.joints > 0 && c.bridges == 5);
    // supports first: a file comes after everything it imports or calls
    assert!(step_of(&d, "worker/store/store.go") < step_of(&d, "worker/main.go"));
    assert!(step_of(&d, "native/src/jobs.rs") < step_of(&d, "native/src/commands.rs"));
    assert!(step_of(&d, "api/services/users.py") < step_of(&d, "api/main.py"));
    assert!(step_of(&d, "web/src/api.ts") < step_of(&d, "web/src/app.ts"));
    // every step stays inside one sub-build and names it
    let ids: HashSet<&str> = d.sub_builds.iter().map(|s| s.id.as_str()).collect();
    assert!(d.steps.iter().all(|s| ids.contains(s.sub_build.as_str()) && !s.caption.is_empty() && s.files.len() <= 3));
    // the gaps in the fixture show up as gaps
    assert_eq!(c.gaps, 2);
}

#[test]
fn check_catches_a_design_that_does_not_hold_and_repair_fixes_it() {
    let g = fixture();
    let mut d = build::engine_design(&g);
    d.source = "claude".into();
    // An imaginative designer: builds the Go entry point first, invents a file,
    // repeats one, and forgets another.
    let main_go = step_of(&d, "worker/main.go");
    let mut first = d.steps.remove(main_go);
    first.files.push("worker/imaginary.go".into());
    d.steps.insert(0, first);
    d.steps[1].files.push("worker/main.go".into());
    let users = step_of(&d, "api/services/users.py");
    d.steps[users].files.retain(|f| f != "api/services/users.py");

    let bad = build::check(&g, &d);
    assert!(!bad.ok);
    let kinds: HashSet<&str> = bad.weak.iter().map(|w| w.kind).collect();
    for k in ["early", "unknown", "duplicate", "missing"] {
        assert!(kinds.contains(k), "expected a `{k}` joint in {:#?}", bad.weak);
    }

    let (fixed, notes) = build::repair(&g, &d);
    let ok = build::check(&g, &fixed);
    assert!(ok.ok, "still weak after repair: {:#?}\nnotes: {notes:#?}", ok.weak);
    assert!(notes.iter().any(|n| n.contains("moved worker/main.go")), "{notes:#?}");
    assert!(notes.iter().any(|n| n.contains("imaginary.go")));
    assert!(notes.iter().any(|n| n.contains("added api/services/users.py")));
    // the designer's words survive the repair
    assert_eq!(fixed.title, d.title);
    assert_eq!(fixed.source, "claude");
}

#[test]
fn geometry_gives_every_symbol_a_brick_and_every_flow_a_bridge() {
    let g = fixture();
    let b = build::assemble(&g, build::engine_design(&g));
    let m = &b.model;
    assert_eq!(m.buildings.len(), 12);
    assert_eq!(m.districts.len(), b.design.sub_builds.len());
    assert!(m.studs.0.is_multiple_of(2) && m.studs.1.is_multiple_of(2) && m.studs.0 >= 12);
    // every district sits on the plate with water around it
    for d in &m.districts {
        assert!(d.x >= 2 && d.z >= 2 && d.x as u32 + d.w <= m.studs.0 && d.z as u32 + d.d <= m.studs.1, "{} is off the plate", d.name);
    }
    // each symbol sits in exactly one brick
    let mut seen = HashSet::new();
    for br in &m.bricks {
        for n in &br.nodes {
            assert!(seen.insert(*n), "node {n} is in two bricks");
        }
    }
    let symbols = g.nodes.iter().filter(|n| n.kind == terrarium_core::NodeKind::Symbol).count();
    assert_eq!(m.bricks.iter().filter(|br| br.kind.is_some() || g.node(br.nodes[0]).kind == terrarium_core::NodeKind::Symbol).map(|br| br.nodes.len()).sum::<usize>(), symbols);
    // buildings stay inside their district and never overlap
    for (i, a) in m.buildings.iter().enumerate() {
        let d = &m.districts[a.district];
        assert!(a.x >= d.x && a.z >= d.z && a.x + a.w as i32 <= d.x + d.w as i32 && a.z + a.d as i32 <= d.z + d.d as i32, "{} spills out of {}", a.path, d.name);
        for c in &m.buildings[i + 1..] {
            let apart = a.x + a.w as i32 <= c.x || c.x + c.w as i32 <= a.x || a.z + a.d as i32 <= c.z || c.z + c.d as i32 <= a.z;
            assert!(apart, "{} overlaps {}", a.path, c.path);
        }
    }
    // the ipc bridge joins the scanRepo brick to the scan_repo brick
    let name = |i: usize| g.node(m.bricks[i].nodes[0]).path.clone();
    assert!(m.bridges.iter().any(|br| name(br.from) == "web/src/api.ts#scanRepo" && name(br.to) == "native/src/commands.rs#scan_repo" && br.label == "ipc scan_repo"));
    assert_eq!(m.bridges.len(), 5);
    // a bridge appears once both ends are built
    for br in &m.bridges {
        let (a, c) = (&m.buildings[m.bricks[br.from].building], &m.buildings[m.bricks[br.to].building]);
        assert_eq!(br.step, a.step.max(c.step));
    }
    // the file that starts the longest trace carries a lamp
    assert!(m.buildings.iter().any(|b| b.path == "web/src/app.ts" && b.lamp));
}
