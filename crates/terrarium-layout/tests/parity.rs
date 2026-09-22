use terrarium_layout::{Backend, Input, Params, layout};

fn ring_of_clusters(clusters: u32, per: u32) -> Input {
    let mut edges = Vec::new();
    let mut groups = Vec::new();
    for c in 0..clusters {
        for i in 0..per {
            let id = c * per + i;
            groups.push(c);
            if i > 0 {
                edges.push((id - 1, id, 1.0));
            }
        }
        // one bridge edge to the next cluster
        edges.push((c * per, ((c + 1) % clusters) * per, 0.5));
    }
    Input {
        positions: vec![],
        edges,
        groups,
        mass: vec![],
    }
}

fn mean_edge_len(input: &Input, pos: &[[f32; 2]]) -> f32 {
    let s: f32 = input
        .edges
        .iter()
        .map(|&(a, b, _)| {
            let (p, q) = (pos[a as usize], pos[b as usize]);
            ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2)).sqrt()
        })
        .sum();
    s / input.edges.len() as f32
}

fn mean_pair_len(pos: &[[f32; 2]]) -> f32 {
    let mut s = 0.0;
    let mut c = 0;
    for i in (0..pos.len()).step_by(7) {
        for j in (i + 1..pos.len()).step_by(11) {
            s += ((pos[i][0] - pos[j][0]).powi(2) + (pos[i][1] - pos[j][1]).powi(2)).sqrt();
            c += 1;
        }
    }
    s / c as f32
}

#[test]
fn cpu_layout_pulls_edges_together() {
    let input = ring_of_clusters(4, 25);
    let (pos, report) = layout(&input, Params::default(), Backend::Cpu, 300).unwrap();
    assert_eq!(report.backend, "cpu");
    assert!(pos.iter().all(|p| p[0].is_finite() && p[1].is_finite()));
    assert!(
        mean_edge_len(&input, &pos) * 1.5 < mean_pair_len(&pos),
        "edges {} vs pairs {}",
        mean_edge_len(&input, &pos),
        mean_pair_len(&pos)
    );
}

#[test]
fn gpu_matches_cpu_statistically() {
    let input = ring_of_clusters(4, 25);
    let (cpu, _) = layout(&input, Params::default(), Backend::Cpu, 300).unwrap();
    match layout(&input, Params::default(), Backend::Gpu, 300) {
        Ok((gpu, report)) => {
            assert_eq!(report.backend, "gpu");
            assert!(gpu.iter().all(|p| p[0].is_finite() && p[1].is_finite()));
            let (a, b) = (mean_edge_len(&input, &cpu), mean_edge_len(&input, &gpu));
            let ratio = a / b;
            assert!((0.6..1.6).contains(&ratio), "cpu edge len {a} vs gpu {b}");
        }
        Err(e) => eprintln!("GPU unavailable in this environment: {e}"),
    }
}

#[test]
fn auto_reports_a_backend() {
    let input = ring_of_clusters(2, 10);
    let (_, report) = layout(&input, Params::default(), Backend::Auto, 10).unwrap();
    assert!(report.backend == "gpu" || report.backend == "cpu");
    assert!(report.ms >= 0.0);
}
