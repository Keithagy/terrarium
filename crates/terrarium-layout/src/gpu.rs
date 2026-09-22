use crate::{Engine, GpuInfo, Input, Params, Prepared};
use anyhow::{Context, anyhow};
use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuParams {
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
}

pub struct GpuEngine {
    device: wgpu::Device,
    queue: wgpu::Queue,
    adapter_name: String,
    n: u32,
    groups: u32,
    pos: [wgpu::Buffer; 2],
    vel: wgpu::Buffer,
    staging: wgpu::Buffer,
    bind_groups: [wgpu::BindGroup; 2],
    centroid_pipeline: wgpu::ComputePipeline,
    force_pipeline: wgpu::ComputePipeline,
    /// which of `pos` currently holds the latest positions
    cur: usize,
}

fn adapter() -> anyhow::Result<(wgpu::Adapter, wgpu::Instance)> {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::METAL | wgpu::Backends::PRIMARY,
        ..Default::default()
    });
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
    }))
    .context("no compatible GPU adapter")?;
    Ok((adapter, instance))
}

pub fn probe() -> GpuInfo {
    match adapter() {
        Ok((a, _)) => {
            let info = a.get_info();
            GpuInfo {
                available: true,
                adapter: info.name,
                backend: format!("{:?}", info.backend),
                device_type: format!("{:?}", info.device_type),
            }
        }
        Err(e) => GpuInfo {
            available: false,
            adapter: e.to_string(),
            backend: "none".into(),
            device_type: "none".into(),
        },
    }
}

impl GpuEngine {
    pub fn new(input: &Input, params: Params) -> anyhow::Result<Self> {
        let p = Prepared::new(input);
        if p.n == 0 {
            return Err(anyhow!("empty graph"));
        }
        let (adapter, _instance) = adapter()?;
        let adapter_name = adapter.get_info().name;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("terrarium-layout"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            memory_hints: wgpu::MemoryHints::Performance,
            trace: wgpu::Trace::Off,
        }))
        .context("cannot create GPU device")?;

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("force.wgsl"),
            source: wgpu::ShaderSource::Wgsl(include_str!("force.wgsl").into()),
        });

        let gp = GpuParams {
            n: p.n as u32,
            groups: p.group_count,
            repulsion: params.repulsion,
            attraction: params.attraction,
            gravity: params.gravity,
            cluster_gravity: params.cluster_gravity,
            damping: params.damping,
            dt: params.dt,
            ideal: params.ideal,
            cutoff: params.cutoff,
            max_speed: params.max_speed,
            _pad: 0.0,
        };
        let mk = |label: &str, data: &[u8], usage: wgpu::BufferUsages| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents: data,
                usage,
            })
        };
        let storage = wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_SRC
            | wgpu::BufferUsages::COPY_DST;
        let pos_bytes = bytemuck::cast_slice::<[f32; 2], u8>(&p.positions);
        let pos = [
            mk("pos0", pos_bytes, storage),
            mk("pos1", pos_bytes, storage),
        ];
        let vel = mk("vel", &vec![0u8; pos_bytes.len()], storage);
        let uniform = mk(
            "params",
            bytemuck::bytes_of(&gp),
            wgpu::BufferUsages::UNIFORM,
        );
        let meta_v: Vec<[f32; 2]> = p
            .mass
            .iter()
            .zip(&p.groups)
            .map(|(m, g)| [*m, *g as f32])
            .collect();
        let meta = mk("meta", bytemuck::cast_slice(&meta_v), storage);
        let offsets = mk("offsets", bytemuck::cast_slice(&p.offsets), storage);
        // WGSL forbids zero-length arrays; pad with one entry.
        let mut adj_v: Vec<[f32; 2]> = p
            .targets
            .iter()
            .zip(&p.weights)
            .map(|(t, w)| [*t as f32, *w])
            .collect();
        if adj_v.is_empty() {
            adj_v.push([0.0, 0.0]);
        }
        let adj = mk("adj", bytemuck::cast_slice(&adj_v), storage);
        let centroids = mk("centroids", &vec![0u8; 8 * p.group_count as usize], storage);
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("staging"),
            size: pos_bytes.len() as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let entry = |binding: u32, ty: wgpu::BufferBindingType| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        use wgpu::BufferBindingType::{Storage, Uniform};
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("layout-bgl"),
            entries: &[
                entry(0, Uniform),
                entry(1, Storage { read_only: true }),
                entry(2, Storage { read_only: false }),
                entry(3, Storage { read_only: false }),
                entry(4, Storage { read_only: true }),
                entry(5, Storage { read_only: true }),
                entry(6, Storage { read_only: true }),
                entry(7, Storage { read_only: false }),
            ],
        });
        let bind = |input: &wgpu::Buffer, output: &wgpu::Buffer| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("layout-bg"),
                layout: &layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: uniform.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: input.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: output.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: vel.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: meta.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 5,
                        resource: offsets.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 6,
                        resource: adj.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 7,
                        resource: centroids.as_entire_binding(),
                    },
                ],
            })
        };
        let bind_groups = [bind(&pos[0], &pos[1]), bind(&pos[1], &pos[0])];
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("layout-pl"),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let make_pipeline = |entry: &str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(&pipeline_layout),
                module: &shader,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let centroid_pipeline = make_pipeline("centroids_main");
        let force_pipeline = make_pipeline("forces_main");
        Ok(Self {
            device,
            queue,
            adapter_name,
            n: p.n as u32,
            groups: p.group_count,
            pos,
            vel,
            staging,
            bind_groups,
            centroid_pipeline,
            force_pipeline,
            cur: 0,
        })
    }

    fn read_buffer(&self, buf: &wgpu::Buffer) -> anyhow::Result<Vec<[f32; 2]>> {
        let mut enc = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("readback"),
            });
        enc.copy_buffer_to_buffer(buf, 0, &self.staging, 0, self.staging.size());
        self.queue.submit(Some(enc.finish()));
        let slice = self.staging.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        self.device
            .poll(wgpu::PollType::Wait)
            .context("device poll")?;
        rx.recv().context("map channel")?.context("map failed")?;
        let data = slice.get_mapped_range();
        let out: Vec<[f32; 2]> = bytemuck::cast_slice(&data).to_vec();
        drop(data);
        self.staging.unmap();
        Ok(out)
    }
}

impl Engine for GpuEngine {
    fn step(&mut self, iterations: u32) -> anyhow::Result<()> {
        // Batch dispatches so the CPU never waits between iterations.
        const BATCH: u32 = 64;
        let mut remaining = iterations;
        while remaining > 0 {
            let chunk = remaining.min(BATCH);
            let mut enc = self
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("layout-step"),
                });
            for _ in 0..chunk {
                let bg = &self.bind_groups[self.cur];
                {
                    let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                        label: Some("centroids"),
                        timestamp_writes: None,
                    });
                    pass.set_pipeline(&self.centroid_pipeline);
                    pass.set_bind_group(0, bg, &[]);
                    pass.dispatch_workgroups(self.groups.div_ceil(64), 1, 1);
                }
                {
                    let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                        label: Some("forces"),
                        timestamp_writes: None,
                    });
                    pass.set_pipeline(&self.force_pipeline);
                    pass.set_bind_group(0, bg, &[]);
                    pass.dispatch_workgroups(self.n.div_ceil(64), 1, 1);
                }
                self.cur ^= 1;
            }
            self.queue.submit(Some(enc.finish()));
            remaining -= chunk;
        }
        Ok(())
    }

    fn positions(&mut self) -> anyhow::Result<Vec<[f32; 2]>> {
        let buf = &self.pos[self.cur];
        self.read_buffer(buf)
    }

    fn energy(&mut self) -> anyhow::Result<f32> {
        let v = self.read_buffer(&self.vel)?;
        Ok(crate::energy(&v))
    }

    fn backend(&self) -> &'static str {
        "gpu"
    }

    fn adapter(&self) -> String {
        self.adapter_name.clone()
    }
}
