//! GFPE sur GPU (wgpu + WGSL) : un lot de calculs de même fréquence (un par
//! azimut) avance en parallèle, un groupe de travail par azimut. Discrétisation
//! partagée avec le moteur CPU (`sm_core::gfpe::GfpeGrid`), calcul en f32.
//!
//! Bascule automatique : [`GpuGfpe::new`] renvoie `None` sans adaptateur
//! compatible, et l'appelant garde alors le moteur CPU.

use bytemuck::{Pod, Zeroable};
use sm_core::gfpe::{GfpeGrid, GfpeInput, GfpeOutput, starter, step_params};
use std::num::NonZeroU64;
use thiserror::Error;
use wgpu::util::DeviceExt;

#[derive(Debug, Error)]
pub enum GpuError {
    #[error("lot vide")]
    Empty,
    #[error("lot hétérogène : {0}")]
    Mismatch(&'static str),
    #[error("lot trop grand pour le GPU ({0} octets)")]
    TooLarge(u64),
    #[error("lecture du résultat GPU : {0}")]
    Readback(String),
    #[error("calcul annulé")]
    Cancelled,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Params {
    n: u32,
    m: u32,
    log2m: u32,
    batches: u32,
    steps: u32,
    _p: [u32; 3],
    dz: f32,
    norm: f32,
    hr: f32,
    zs: f32,
    tw_c: [f32; 2],
    r0: f32,
    dr: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Spec {
    kz: f32,
    _pad: f32,
    tw_k: [f32; 2],
    kzt: [f32; 2],
    tw_z: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct StepP {
    rot: f32,
    ds: f32,
    beta: [f32; 2],
    p_beta: [f32; 2],
    _pad: [f32; 2],
}

/// Pas soumis par lot de commandes (vérification d'annulation entre deux).
const STEPS_PER_SUBMIT: usize = 64;

/// Moteur GFPE GPU (device, file et pipeline réutilisables).
pub struct GpuGfpe {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    name: String,
    max_binding: u64,
    uniform_align: u64,
}

fn c2(v: num_complex::Complex64) -> [f32; 2] {
    [v.re as f32, v.im as f32]
}

impl GpuGfpe {
    /// Initialise wgpu ; `None` si aucun adaptateur ne gère les compute shaders.
    pub fn new() -> Option<Self> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }))
        .ok()?;
        if !adapter
            .get_downlevel_capabilities()
            .flags
            .contains(wgpu::DownlevelFlags::COMPUTE_SHADERS)
        {
            return None;
        }
        let info = adapter.get_info();
        if info.device_type == wgpu::DeviceType::Cpu {
            // Rendu logiciel (llvmpipe, WARP) : plus lent que le moteur CPU.
            return None;
        }
        let alimits = adapter.limits();
        let limits = wgpu::Limits {
            max_storage_buffer_binding_size: alimits.max_storage_buffer_binding_size,
            max_buffer_size: alimits.max_buffer_size,
            ..wgpu::Limits::default()
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("sm-gpu"),
            required_features: wgpu::Features::empty(),
            required_limits: limits.clone(),
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            memory_hints: wgpu::MemoryHints::Performance,
            trace: wgpu::Trace::Off,
        }))
        .ok()?;
        let module = device.create_shader_module(wgpu::include_wgsl!("gfpe.wgsl"));
        let storage = |binding: u32, read_only: bool| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gfpe"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: NonZeroU64::new(16),
                    },
                    count: None,
                },
                storage(2, true),
                storage(3, true),
                storage(4, true),
                storage(5, false),
                storage(6, false),
                storage(7, false),
                storage(8, true),
            ],
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("gfpe"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("gfpe"),
            layout: Some(&pl),
            module: &module,
            entry_point: Some("step_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });
        Some(Self {
            device,
            queue,
            pipeline,
            layout,
            name: format!("{} ({:?})", info.name, info.backend),
            max_binding: limits.max_storage_buffer_binding_size,
            uniform_align: u64::from(limits.min_uniform_buffer_offset_alignment),
        })
    }

    /// Nom de l'adaptateur (pour l'affichage de la méthode utilisée).
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Nombre maximal de calculs par lot pour une grille donnée (mémoire GPU).
    pub fn max_batch(&self, grid: &GfpeGrid) -> usize {
        let per = 4 * grid.m as u64 * 8;
        ((self.max_binding / per.max(1)) as usize).max(1)
    }

    /// Calcule un lot de GFPE de même fréquence, portée, hauteur de source et
    /// récepteur unique (un par azimut). Résultat dans l'ordre des entrées.
    pub fn run_batch(
        &self,
        inputs: &[GfpeInput],
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Vec<GfpeOutput>, GpuError> {
        let first = inputs.first().ok_or(GpuError::Empty)?;
        let g = GfpeGrid::new(first);
        let hr = *first
            .receivers
            .first()
            .ok_or(GpuError::Mismatch("récepteur"))?;
        for i in inputs {
            if GfpeGrid::new(i) != g {
                return Err(GpuError::Mismatch("grille"));
            }
            if i.zs != first.zs || i.receivers.first() != Some(&hr) {
                return Err(GpuError::Mismatch("hauteurs"));
            }
        }
        let nb = inputs.len();
        let (n, m, steps) = (g.n, g.m, g.steps);
        let work_bytes = (nb * 4 * m * 8) as u64;
        if work_bytes > self.max_binding {
            return Err(GpuError::TooLarge(work_bytes));
        }

        // Données constantes et par lot (f64 → f32).
        let tw_c = g.tw_c();
        let spec: Vec<Spec> = (0..m)
            .map(|i| {
                let (tk, tz) = g.twiddles(i);
                Spec {
                    kz: g.kz(i) as f32,
                    _pad: 0.0,
                    tw_k: c2(tk),
                    kzt: c2(g.kz_term(i)),
                    tw_z: c2(tz),
                }
            })
            .collect();
        let tw: Vec<[f32; 2]> = (0..m / 2)
            .map(|q| {
                c2(num_complex::Complex64::from_polar(
                    1.0,
                    -2.0 * std::f64::consts::PI * q as f64 / m as f64,
                ))
            })
            .collect();
        let mut sp = vec![StepP::zeroed(); steps * nb];
        let mut dk = vec![[0f32; 2]; nb * n];
        let mut psi = vec![[0f32; 2]; nb * n];
        for (b, input) in inputs.iter().enumerate() {
            for (s, p) in step_params(input, &g).into_iter().enumerate() {
                sp[s * nb + b] = StepP {
                    rot: p.rot as f32,
                    ds: p.ds as f32,
                    beta: c2(p.beta),
                    p_beta: c2(p.p_beta),
                    _pad: [0.0; 2],
                };
            }
            for j in 0..n {
                dk[b * n + j] = c2(g.delta_k(input, j));
            }
            for (j, v) in starter(input, &g).into_iter().enumerate() {
                psi[b * n + j] = c2(v);
            }
        }
        let params = Params {
            n: n as u32,
            m: m as u32,
            log2m: m.trailing_zeros(),
            batches: nb as u32,
            steps: steps as u32,
            _p: [0; 3],
            dz: g.dz as f32,
            norm: (1.0 / m as f64) as f32,
            hr: hr as f32,
            zs: first.zs as f32,
            tw_c: c2(tw_c),
            r0: g.r0 as f32,
            dr: g.dr as f32,
        };
        let stride = self.uniform_align.max(16);
        let mut step_bytes = vec![0u8; steps * stride as usize];
        for s in 0..steps {
            let o = s * stride as usize;
            step_bytes[o..o + 4].copy_from_slice(&(s as u32).to_le_bytes());
        }

        let d = &self.device;
        let init = |label: &str, contents: &[u8], usage: wgpu::BufferUsages| {
            d.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents,
                usage,
            })
        };
        let st = wgpu::BufferUsages::STORAGE;
        let b_params = init(
            "params",
            bytemuck::bytes_of(&params),
            wgpu::BufferUsages::UNIFORM,
        );
        let b_step = init("step", &step_bytes, wgpu::BufferUsages::UNIFORM);
        let b_spec = init("spec", bytemuck::cast_slice(&spec), st);
        let b_sp = init("steps", bytemuck::cast_slice(&sp), st);
        let b_dk = init("dk", bytemuck::cast_slice(&dk), st);
        let b_psi = init("psi", bytemuck::cast_slice(&psi), st);
        let b_tw = init("tw", bytemuck::cast_slice(&tw), st);
        let b_work = d.create_buffer(&wgpu::BufferDescriptor {
            label: Some("work"),
            size: work_bytes,
            usage: st,
            mapped_at_creation: false,
        });
        let out_bytes = (nb * steps * 4) as u64;
        let b_out = d.create_buffer(&wgpu::BufferDescriptor {
            label: Some("out"),
            size: out_bytes,
            usage: st | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let b_read = d.create_buffer(&wgpu::BufferDescriptor {
            label: Some("read"),
            size: out_bytes,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        fn entry(binding: u32, buffer: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
            wgpu::BindGroupEntry {
                binding,
                resource: buffer.as_entire_binding(),
            }
        }
        let bind = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gfpe"),
            layout: &self.layout,
            entries: &[
                entry(0, &b_params),
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &b_step,
                        offset: 0,
                        size: NonZeroU64::new(16),
                    }),
                },
                entry(2, &b_spec),
                entry(3, &b_sp),
                entry(4, &b_dk),
                entry(5, &b_psi),
                entry(6, &b_work),
                entry(7, &b_out),
                entry(8, &b_tw),
            ],
        });

        let mut s0 = 0;
        while s0 < steps {
            if cancelled() {
                return Err(GpuError::Cancelled);
            }
            let s1 = (s0 + STEPS_PER_SUBMIT).min(steps);
            let mut enc = d.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("gfpe"),
            });
            for s in s0..s1 {
                let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: None,
                    timestamp_writes: None,
                });
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &bind, &[(s as u64 * stride) as u32]);
                pass.dispatch_workgroups(nb as u32, 1, 1);
            }
            if s1 == steps {
                enc.copy_buffer_to_buffer(&b_out, 0, &b_read, 0, out_bytes);
            }
            self.queue.submit([enc.finish()]);
            // Attend la fin du lot (évite d'empiler des secondes de travail GPU).
            d.poll(wgpu::PollType::wait_indefinitely())
                .map_err(|e| GpuError::Readback(e.to_string()))?;
            s0 = s1;
        }
        let slice = b_read.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        d.poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| GpuError::Readback(e.to_string()))?;
        let levels: Vec<f32> = {
            let data = slice
                .get_mapped_range()
                .map_err(|e| GpuError::Readback(e.to_string()))?;
            bytemuck::allocation::pod_collect_to_vec(&data)
        };
        b_read.unmap();
        let r: Vec<f64> = (1..=steps).map(|s| g.r_of(s)).collect();
        Ok((0..nb)
            .map(|b| GfpeOutput {
                r: r.clone(),
                dl: vec![levels[b * steps..(b + 1) * steps].to_vec()],
                dz: g.dz,
                dr: g.dr,
                n,
            })
            .collect())
    }
}
