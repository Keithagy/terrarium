// Force-directed layout, one thread per node.
//
// Forces (all in 2D):
//   repulsion  every pair, k_r * m_i * m_j / d^2, cut off beyond `cutoff`
//   attraction every edge, spring toward ideal length scaled by weight
//   gravity    toward the origin (keeps disconnected pieces on screen)
//   cluster    toward the centroid of the node's group (packages stay together)

struct Params {
    n: u32,
    groups: u32,
    repulsion: f32,
    attraction: f32,
    gravity: f32,
    cluster_gravity: f32,
    damping: f32,
    dt: f32,
    ideal: f32,
    cutoff: f32,
    max_speed: f32,
    _pad: f32,
};

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> pos_in: array<vec2<f32>>;
@group(0) @binding(2) var<storage, read_write> pos_out: array<vec2<f32>>;
@group(0) @binding(3) var<storage, read_write> vel: array<vec2<f32>>;
// Packed to stay under the 8-storage-buffer limit: node_meta.x = mass, node_meta.y = group id.
@group(0) @binding(4) var<storage, read> node_meta: array<vec2<f32>>;
@group(0) @binding(5) var<storage, read> offsets: array<u32>;      // CSR, n + 1 entries
@group(0) @binding(6) var<storage, read> adj: array<vec2<f32>>;    // CSR: x = neighbour id, y = weight
@group(0) @binding(7) var<storage, read_write> centroids: array<vec2<f32>>; // per group

fn mass_of(i: u32) -> f32 { return node_meta[i].x; }
// -1 means "no cluster": the node is held only by its edges and global gravity.
fn group_of(i: u32) -> i32 { return i32(node_meta[i].y); }

// Pass 1: group centroids (one thread per group).
@compute @workgroup_size(64)
fn centroids_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let g = gid.x;
    if (g >= params.groups) { return; }
    var sum = vec2<f32>(0.0, 0.0);
    var count = 0.0;
    for (var i = 0u; i < params.n; i = i + 1u) {
        if (group_of(i) == i32(g)) {
            sum = sum + pos_in[i];
            count = count + 1.0;
        }
    }
    if (count > 0.0) {
        centroids[g] = sum / count;
    }
}

// Pass 2: forces + integration (one thread per node).
@compute @workgroup_size(64)
fn forces_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= params.n) { return; }
    let p = pos_in[i];
    let mi = mass_of(i);
    var f = vec2<f32>(0.0, 0.0);

    // repulsion
    let cutoff2 = params.cutoff * params.cutoff;
    for (var j = 0u; j < params.n; j = j + 1u) {
        if (j == i) { continue; }
        var d = p - pos_in[j];
        var d2 = dot(d, d);
        if (d2 < 1e-4) {
            // coincident: nudge deterministically by index
            d = vec2<f32>(f32(i % 7u) - 3.0, f32(j % 5u) - 2.0) * 0.01 + vec2<f32>(0.013, -0.007);
            d2 = dot(d, d);
        }
        if (d2 > cutoff2) { continue; }
        let inv = 1.0 / d2;
        f = f + d * (params.repulsion * mi * mass_of(j) * inv * inverseSqrt(d2));
    }

    // attraction along edges
    let start = offsets[i];
    let end = offsets[i + 1u];
    for (var e = start; e < end; e = e + 1u) {
        let j = u32(adj[e].x);
        let d = pos_in[j] - p;
        let dist = max(length(d), 1e-3);
        let ideal = params.ideal * (0.5 + 0.5 * (mi + mass_of(j)) * 0.5);
        let stretch = (dist - ideal) / ideal;
        f = f + d / dist * (params.attraction * adj[e].y * stretch);
    }

    // gravity to the origin and to the group centroid
    f = f - p * params.gravity;
    let gi = group_of(i);
    if (gi >= 0) {
        let c = centroids[u32(gi)];
        f = f + (c - p) * params.cluster_gravity;
    }

    // integrate (semi-implicit Euler with damping and a speed cap)
    var v = (vel[i] + f * params.dt / max(mi, 0.25)) * params.damping;
    let speed = length(v);
    if (speed > params.max_speed) {
        v = v * (params.max_speed / speed);
    }
    vel[i] = v;
    pos_out[i] = p + v * params.dt;
}
