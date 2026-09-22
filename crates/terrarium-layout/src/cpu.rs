use crate::{Engine, Input, Params, Prepared};
use rayon::prelude::*;

pub struct CpuEngine {
    p: Prepared,
    params: Params,
    vel: Vec<[f32; 2]>,
    centroids: Vec<[f32; 2]>,
}

impl CpuEngine {
    pub fn new(input: &Input, params: Params) -> Self {
        let p = Prepared::new(input);
        let n = p.n;
        Self {
            vel: vec![[0.0; 2]; n],
            centroids: vec![[0.0; 2]; p.group_count as usize],
            p,
            params,
        }
    }

    fn one_step(&mut self) {
        let pr = &self.p;
        let prm = self.params;
        // centroids
        let mut sums = vec![([0.0f32, 0.0f32], 0u32); pr.group_count as usize];
        for i in 0..pr.n {
            if pr.groups[i] == crate::NO_GROUP {
                continue;
            }
            let g = pr.groups[i] as usize;
            sums[g].0[0] += pr.positions[i][0];
            sums[g].0[1] += pr.positions[i][1];
            sums[g].1 += 1;
        }
        for (g, (s, c)) in sums.iter().enumerate() {
            if *c > 0 {
                self.centroids[g] = [s[0] / *c as f32, s[1] / *c as f32];
            }
        }
        let pos = &pr.positions;
        let centroids = &self.centroids;
        let cutoff2 = prm.cutoff * prm.cutoff;
        #[allow(clippy::needless_range_loop)]
        let new: Vec<([f32; 2], [f32; 2])> = (0..pr.n)
            .into_par_iter()
            .map(|i| {
                let p = pos[i];
                let mi = pr.mass[i];
                let mut f = [0.0f32, 0.0f32];
                for j in 0..pr.n {
                    if j == i {
                        continue;
                    }
                    let mut d = [p[0] - pos[j][0], p[1] - pos[j][1]];
                    let mut d2 = d[0] * d[0] + d[1] * d[1];
                    if d2 < 1e-4 {
                        d = [(i % 7) as f32 - 3.0, (j % 5) as f32 - 2.0];
                        d = [d[0] * 0.01 + 0.013, d[1] * 0.01 - 0.007];
                        d2 = d[0] * d[0] + d[1] * d[1];
                    }
                    if d2 > cutoff2 {
                        continue;
                    }
                    let inv = 1.0 / d2;
                    let s = prm.repulsion * mi * pr.mass[j] * inv / d2.sqrt();
                    f[0] += d[0] * s;
                    f[1] += d[1] * s;
                }
                for e in pr.offsets[i]..pr.offsets[i + 1] {
                    let j = pr.targets[e as usize] as usize;
                    let d = [pos[j][0] - p[0], pos[j][1] - p[1]];
                    let dist = (d[0] * d[0] + d[1] * d[1]).sqrt().max(1e-3);
                    let ideal = prm.ideal * (0.5 + 0.5 * (mi + pr.mass[j]) * 0.5);
                    let stretch = (dist - ideal) / ideal;
                    let s = prm.attraction * pr.weights[e as usize] * stretch / dist;
                    f[0] += d[0] * s;
                    f[1] += d[1] * s;
                }
                f[0] -= p[0] * prm.gravity;
                f[1] -= p[1] * prm.gravity;
                if pr.groups[i] != crate::NO_GROUP {
                    let c = centroids[pr.groups[i] as usize];
                    f[0] += (c[0] - p[0]) * prm.cluster_gravity;
                    f[1] += (c[1] - p[1]) * prm.cluster_gravity;
                }
                let m = mi.max(0.25);
                let mut v = [
                    (self.vel[i][0] + f[0] * prm.dt / m) * prm.damping,
                    (self.vel[i][1] + f[1] * prm.dt / m) * prm.damping,
                ];
                let speed = (v[0] * v[0] + v[1] * v[1]).sqrt();
                if speed > prm.max_speed {
                    let k = prm.max_speed / speed;
                    v = [v[0] * k, v[1] * k];
                }
                (v, [p[0] + v[0] * prm.dt, p[1] + v[1] * prm.dt])
            })
            .collect();
        for (i, (v, p)) in new.into_iter().enumerate() {
            self.vel[i] = v;
            self.p.positions[i] = p;
        }
    }
}

impl Engine for CpuEngine {
    fn step(&mut self, iterations: u32) -> anyhow::Result<()> {
        for _ in 0..iterations {
            self.one_step();
        }
        Ok(())
    }
    fn positions(&mut self) -> anyhow::Result<Vec<[f32; 2]>> {
        Ok(self.p.positions.clone())
    }
    fn energy(&mut self) -> anyhow::Result<f32> {
        Ok(crate::energy(&self.vel))
    }
    fn backend(&self) -> &'static str {
        "cpu"
    }
    fn adapter(&self) -> String {
        format!("rayon ({} threads)", rayon::current_num_threads())
    }
}
