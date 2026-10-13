//! DLSS in the renderer: the motion prepass's pipelines, the targets Streamline reads and
//! writes, the camera's sub-pixel jitter and last frame's camera, and the present of its result
//! with the HUD over it. The runtime itself (Streamline on wgpu's D3D12 device) is `dlss`.
//!
//! Kept apart from the frame's passes, which only call in here where DLSS changes them: the
//! frame's size and targets (`begin_frame`), the projection (`frame_uniforms`), the depth
//! prepass's motion colours (`encode_depth_prepass`), and the end of the picture
//! (`encode_upscale`, then `dlss_present` after the submit).
//!
//! After openOMSI PR #942 (DLSS Super Resolution and DLAA through NVIDIA Streamline, by
//! Nussaccino, MIT), brought onto the renderer as it is split into modules since.

use crate::pipelines::common::compile_pipeline;
use crate::passes::FrameCtx;
use crate::pipelines::scene::SceneBase;
use crate::*;

/// DLSS's motion vectors: pixels of the render size from each pixel to where its point was
/// last frame.
pub(crate) const DLSS_MOTION_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rg32Float;
/// DLSS's result at the window's size (a storage texture: sRGB formats cannot be one).
pub(crate) const DLSS_OUTPUT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// DLSS's `BiasCurrentColor` mask: 1 where the current picture goes before the history (the
/// rain films' drops).
pub(crate) const DLSS_BIAS_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R8Unorm;

/// The scene module carries the motion passes (`dlss_motion.wgsl`): set before the scene
/// shader is compiled for a renderer that asks for DLSS (see `Renderer::build`).
pub(crate) static DLSS_MOTION: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The scene module's text with the motion passes after it, when they are asked for and the
/// arrays are storage buffers (DLSS draws on DirectX 12 only); else the text as it is.
pub(crate) fn with_motion_passes(mut src: String, gl: bool, path: ArrayPath) -> String {
    if !gl && path == ArrayPath::Storage && DLSS_MOTION.load(std::sync::atomic::Ordering::Relaxed) {
        src.push('\n');
        src.push_str(include_str!("dlss_motion.wgsl"));
    }
    src
}

/// `Motion` in dlss_motion.wgsl.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct MotionUniform {
    vp: [[f32; 4]; 4],
    vp_prev: [[f32; 4]; 4],
    vp_prev_here: [[f32; 4]; 4],
    vp_inv: [[f32; 4]; 4],
    params: [f32; 4],
}

/// The DLSS depth prepass: [plain, alpha-tested, transmap] x [two-sided, culled] as the
/// ordinary prepass, writing motion beside depth, and the sky's motion.
pub(crate) struct DlssPipelines {
    prepass: [wgpu::RenderPipeline; 6],
    sky: wgpu::RenderPipeline,
    /// The drops of the rain films: their motion (`fs_motion_rain`) and the bias mask
    /// (`fs_bias_rain`).
    rain: wgpu::RenderPipeline,
    rain_bias: wgpu::RenderPipeline,
    motion_layout: wgpu::BindGroupLayout,
}

/// What DLSS reads and writes for one window size and mode.
pub(crate) struct DlssTargets {
    render: (u32, u32),
    output: (u32, u32),
    /// The 3D picture at the render size (the renderer's format), with the upscale bind group
    /// the rain on the glass reads it by.
    color: wgpu::Texture,
    color_view: wgpu::TextureView,
    color_bg: wgpu::BindGroup,
    motion: wgpu::Texture,
    motion_view: wgpu::TextureView,
    /// DLSS's `BiasCurrentColor` mask.
    bias: wgpu::Texture,
    bias_view: wgpu::TextureView,
    /// DLSS's result, and the bind group that draws it into the window.
    output_tex: wgpu::Texture,
    output_bg: wgpu::BindGroup,
    output_view: wgpu::TextureView,
}

/// Last frame's camera, for the motion vectors and DLSS's `clipToPrevClip`.
#[derive(Clone, Copy)]
struct DlssPrev {
    camera: Camera,
    aspect: f32,
    origin: DVec3,
    render: (u32, u32),
    output: (u32, u32),
    mode: DlssMode,
}

/// DLSS once Streamline is up: its runtime, targets, and what the last frame left for the
/// next one's motion vectors.
pub(crate) struct DlssState {
    runtime: dlss::Runtime,
    targets: Option<DlssTargets>,
    /// Textures DLSS read in the last frames: a texture wgpu lets go of when its last submit is
    /// done, and DLSS's command list comes after that submit.
    keep: std::collections::VecDeque<Vec<wgpu::Texture>>,
    motion_buf: wgpu::Buffer,
    /// Blit parameters of the result into the window (`upscale.wgsl` at the window's size).
    blit_buf: wgpu::Buffer,
    /// Last frame's model matrices (copied at the end of each frame) and its bind group with
    /// `motion_buf`.
    prev_models: Option<(wgpu::Buffer, wgpu::BindGroup)>,
    /// The scene's model layout (`Scene::model_epoch`) and entry count the copy was made with.
    prev_epoch: u64,
    prev_entries: u32,
    prev: Option<DlssPrev>,
    frame: u64,
}

/// What a DLSS frame hands on from its camera to DLSS (after the scene's submit).
pub(crate) struct DlssFrame {
    /// This frame's view-projection without the jitter, and last frame's relative to this
    /// frame's render origin.
    vp: Mat4,
    vp_prev_here: Mat4,
    /// The offset the projection was moved by, in pixels.
    jitter: [f32; 2],
    reset: bool,
    aspect: f32,
}

/// The renderer's DLSS: the motion prepass (made when DLSS is asked for), Streamline and its
/// targets once the first frame started it, whether it could not, whether the picture being
/// drawn is the window's own (`render`: the one DLSS is for), and the frame's numbers waiting
/// for `dlss_present` at its end.
#[derive(Default)]
pub(crate) struct DlssRes {
    pub(crate) pipes: Option<DlssPipelines>,
    state: Option<DlssState>,
    failed: bool,
    pub(crate) window: bool,
    pending: Option<DlssFrame>,
}

impl DlssRes {
    pub(crate) fn new(pipes: Option<DlssPipelines>) -> DlssRes {
        DlssRes { pipes, ..Default::default() }
    }

    /// Whether DLSS may draw the window's picture (asked for, its pipelines made, not failed).
    pub(crate) fn usable(&self, options: &RenderOptions) -> bool {
        options.dlss.is_on() && self.pipes.is_some() && !self.failed
    }
}

/// The Halton sequence's `i`th value (from 1) in `base`, 0..1.
pub(crate) fn halton(mut i: u32, base: u32) -> f32 {
    let (mut f, mut r) = (1.0f32, 0.0f32);
    while i > 0 {
        f /= base as f32;
        r += f * (i % base) as f32;
        i /= base;
    }
    r
}

/// Matrices as DLSS takes them: Streamline's rows are glam's columns (row vectors).
pub(crate) fn sl_matrix(m: Mat4) -> [[f32; 4]; 4] {
    m.to_cols_array_2d()
}

/// DLSS's options for a renderer about to be built on this adapter: it takes the place of
/// multisampling, and needs DirectX 12 on an NVIDIA card (whether this one is an RTX with a
/// driver new enough, Streamline says at the first frame, see `DlssState`).
pub(crate) fn settle_options(options: RenderOptions, info: &wgpu::AdapterInfo) -> RenderOptions {
    if options.dlss.is_on() && !(cfg!(windows) && info.backend == wgpu::Backend::Dx12 && info.vendor == 0x10de) {
        log::warn!("DLSS ({:?}) was asked for, but {} on {:?} cannot do it (a GeForce RTX card on DirectX 12 is needed); drawing without it", options.dlss, info.name, info.backend);
        RenderOptions { dlss: DlssMode::Off, ..options }
    } else if options.dlss.is_on() {
        RenderOptions { msaa: 1, ..options }
    } else {
        options
    }
}

/// The motion prepass's pipelines: the same prepass writing each pixel's motion beside its
/// depth, and the sky's motion laid first (see `DlssState`). Only when DLSS is asked for and
/// the scene module has the motion passes (`with_motion_passes`).
pub(crate) fn build_pipelines(device: &wgpu::Device, scene: &SceneBase, options: &RenderOptions) -> Option<DlssPipelines> {
    if !(options.dlss.is_on() && DLSS_MOTION.load(std::sync::atomic::Ordering::Relaxed) && array_path() == ArrayPath::Storage) {
        return None;
    }
    let buffer = |binding, visibility, ty| wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer { ty, has_dynamic_offset: false, min_binding_size: None },
        count: None,
    };
    let motion_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("dlss motion"),
        entries: &[
            buffer(0, wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT, wgpu::BufferBindingType::Uniform),
            buffer(1, wgpu::ShaderStages::VERTEX, wgpu::BufferBindingType::Storage { read_only: true }),
        ],
    });
    let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("dlss prepass"),
        // (group 2 is the vehicle reflection's in the scene module: DLSS's motion is group 3)
        bind_group_layouts: &[Some(&scene.camera_layout), Some(&scene.material_layout), None, Some(&motion_layout)],
        immediate_size: 0,
    });
    let target = |format, write_mask| Some(wgpu::ColorTargetState { format, blend: None, write_mask });
    // (the prepass's two targets: the motion, and the bias mask only the drops write)
    let motion_target = [target(DLSS_MOTION_FORMAT, wgpu::ColorWrites::ALL), target(DLSS_BIAS_FORMAT, wgpu::ColorWrites::empty())];
    let bias_target = [target(DLSS_MOTION_FORMAT, wgpu::ColorWrites::empty()), target(DLSS_BIAS_FORMAT, wgpu::ColorWrites::ALL)];
    let depth = |write: bool, compare| {
        Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(write),
            depth_compare: Some(compare),
            stencil: Default::default(),
            bias: Default::default(),
        })
    };
    let mesh_pipeline = |label, entry: &str, cull: bool, write: bool, targets: &[Option<wgpu::ColorTargetState>]| {
        compile_pipeline(device, &wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(&pl),
            vertex: wgpu::VertexState {
                module: &scene.shader,
                entry_point: Some("vs_motion"),
                buffers: &[scene.vertex_layout.clone()],
                compilation_options: Default::default(),
            },
            primitive: one_sided_primitive(cull),
            depth_stencil: depth(write, wgpu::CompareFunction::GreaterEqual),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &scene.shader,
                entry_point: Some(entry),
                targets,
                compilation_options: Default::default(),
            }),
            multiview_mask: None,
            cache: crate::pipeline_cache::get(device).as_ref(),
        })
    };
    // (the prepass culls exactly as the ordinary one: see `pipelines::scene::prepass`)
    let make = |kind: u8, cull: bool| {
        let entry = ["fs_motion", "fs_motion_test", "fs_motion_transmap"][kind as usize];
        mesh_pipeline("dlss depth prepass", entry, cull, true, &motion_target)
    };
    let sky_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("dlss sky motion"),
        bind_group_layouts: &[None, None, None, Some(&motion_layout)],
        immediate_size: 0,
    });
    let sky = compile_pipeline(device, &wgpu::RenderPipelineDescriptor {
        label: Some("dlss sky motion"),
        layout: Some(&sky_pl),
        vertex: wgpu::VertexState {
            module: &scene.shader,
            entry_point: Some("vs_motion_sky"),
            buffers: &[],
            compilation_options: Default::default(),
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: depth(false, wgpu::CompareFunction::Always),
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: &scene.shader,
            entry_point: Some("fs_motion_sky"),
            targets: &motion_target,
            compilation_options: Default::default(),
        }),
        multiview_mask: None,
        cache: crate::pipeline_cache::get(device).as_ref(),
    });
    // the rain films' drops: their pane's motion over what the prepass left, the depth DLSS
    // reads untouched
    let rain = mesh_pipeline("dlss rain motion", "fs_motion_rain", false, false, &motion_target);
    let rain_bias = mesh_pipeline("dlss rain bias", "fs_bias_rain", false, false, &bias_target);
    Some(DlssPipelines {
        prepass: [make(0, false), make(0, true), make(1, false), make(1, true), make(2, false), make(2, true)],
        sky,
        rain,
        rain_bias,
        motion_layout,
    })
}

impl Renderer {
    /// Another DLSS quality while the game runs: the next frame is drawn at its size (DLSS
    /// on or off is not changed here - its pipelines are made when the renderer is). True
    /// when it was taken.
    pub fn set_dlss_mode(&mut self, mode: DlssMode) -> bool {
        if !(self.options.dlss.is_on() && mode.is_on() && self.dlss.pipes.is_some()) {
            return false;
        }
        if self.options.dlss != mode {
            log::info!("DLSS: {} from now on", mode.as_str());
            self.options.dlss = mode;
        }
        true
    }

    /// Whether DLSS draws this frame: the window's own picture (not a mirror's, an eye's or
    /// one for a texture), with Streamline started at the first one that asks (once; when it
    /// cannot, the game says why and draws as without DLSS).
    pub(crate) fn dlss_frame_wanted(&mut self, with_overlays: bool, projection: Option<Mat4>) -> bool {
        with_overlays && projection.is_none() && self.dlss.window && self.texture_aspect.is_none() && self.dlss_start()
    }

    fn dlss_start(&mut self) -> bool {
        if self.dlss.state.is_some() {
            return true;
        }
        if !self.dlss.usable(&self.options) {
            return false;
        }
        match dlss::Runtime::load(&self.device, &self.queue) {
            Ok(runtime) => {
                let buf = |label: &str, size: u64| {
                    self.device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some(label),
                        size,
                        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    })
                };
                self.dlss.state = Some(DlssState {
                    runtime,
                    targets: None,
                    keep: Default::default(),
                    motion_buf: buf("dlss motion", std::mem::size_of::<MotionUniform>() as u64),
                    blit_buf: buf("dlss blit", 16),
                    prev_models: None,
                    prev_epoch: u64::MAX,
                    prev_entries: 0,
                    prev: None,
                    frame: 0,
                });
                log::info!("DLSS {} on: {}", self.options.dlss.as_str(), self.adapter_name);
                true
            }
            Err(e) => {
                log::warn!("DLSS cannot start: {e:#}; drawing without it");
                self.dlss.failed = true;
                false
            }
        }
    }

    /// A DLSS frame's picture size for a window of this size, its targets made for it, and
    /// the picture's target with the upscale bind group (the rain on the glass reads it by).
    pub(crate) fn dlss_frame_targets(&mut self, full: (u32, u32)) -> ((u32, u32), Option<(wgpu::TextureView, wgpu::BindGroup)>) {
        let mode = self.options.dlss;
        let size = self.dlss.state.as_mut().expect("dlss state").runtime.render_size(mode, full);
        self.dlss_targets(size, full);
        let target = self.dlss.state.as_ref().and_then(|d| d.targets.as_ref()).map(|t| (t.color_view.clone(), t.color_bg.clone()));
        (size, target)
    }

    /// DLSS's targets for this render and window size (made anew when either changes).
    fn dlss_targets(&mut self, render: (u32, u32), output: (u32, u32)) {
        let Some(blit_buf) = self.dlss.state.as_ref().filter(|d| !d.targets.as_ref().is_some_and(|t| t.render == render && t.output == output)).map(|d| d.blit_buf.clone()) else {
            return;
        };
        let tex = |label: &str, (w, h): (u32, u32), format: wgpu::TextureFormat, usage: wgpu::TextureUsages| {
            self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let targets = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
        // (copied from by the rain on the glass, as the render scale's target)
        let color = tex("dlss scene", render, self.format, targets | wgpu::TextureUsages::COPY_SRC);
        let motion = tex("dlss motion vectors", render, DLSS_MOTION_FORMAT, targets);
        let bias = tex("dlss bias mask", render, DLSS_BIAS_FORMAT, targets);
        let output_tex = tex("dlss output", output, DLSS_OUTPUT_FORMAT, targets | wgpu::TextureUsages::STORAGE_BINDING);
        let color_view = color.create_view(&Default::default());
        let motion_view = motion.create_view(&Default::default());
        let bias_view = bias.create_view(&Default::default());
        let output_view = output_tex.create_view(&Default::default());
        let upscale_bg = |buf: &wgpu::Buffer, view: &wgpu::TextureView| {
            self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("dlss upscale"),
                layout: &self.scale.upscale_layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: buf.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(view) },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.tone.post_sampler) },
                ],
            })
        };
        let color_bg = upscale_bg(&self.scale.upscale_buf, &color_view);
        let output_bg = upscale_bg(&blit_buf, &output_view);
        let d = self.dlss.state.as_mut().expect("dlss state");
        // (the old ones may still be read by DLSS's last command list; the history belongs to
        // the old size)
        if let Some(old) = d.targets.take() {
            d.keep.push_back(vec![old.color, old.motion, old.bias, old.output_tex]);
        }
        d.prev = None;
        d.targets = Some(DlssTargets { render, output, color, color_view, color_bg, motion, motion_view, bias, bias_view, output_tex, output_bg, output_view });
    }

    /// A DLSS frame's projection: every frame moved by another sub-pixel offset (Halton 2, 3;
    /// the more DLSS scales up, the more phases), from which DLSS puts the finer picture
    /// together. The motion vectors are worked out without it (`MotionUniform`), and what
    /// DLSS is told after the submit is kept for `dlss_present`.
    pub(crate) fn dlss_jitter(&mut self, scene: &Scene, f: &FrameCtx, vp_plain: Mat4) -> Mat4 {
        self.dlss.pending = None;
        if !f.dlss_frame {
            return vp_plain;
        }
        let (camera, ro, aspect) = (f.camera, f.ro, f.aspect);
        let (width, height, full_w, full_h) = (f.width, f.height, f.full_w, f.full_h);
        let mode = self.options.dlss;
        let Some(d) = self.dlss.state.as_mut() else { return vp_plain };
        d.frame += 1;
        let ratio = (full_w as f32 / width.max(1) as f32).max(1.0);
        let phases = ((8.0 * ratio * ratio).round() as u32).clamp(8, 64);
        let i = (d.frame % phases as u64) as u32 + 1;
        // (OMSI_DLSS_JITTER_SIGN=-1 tells DLSS the offset the other way round: for trying
        // which way this driver reads it)
        let sign = omsi_cfg::flags::OMSI_DLSS_JITTER_SIGN.var().and_then(|v| v.trim().parse::<f32>().ok()).unwrap_or(1.0).signum();
        let jitter = [halton(i, 2) - 0.5, halton(i, 3) - 0.5];
        // (in pixels, x right and y down)
        let shift = Mat4::from_translation(Vec3::new(2.0 * jitter[0] / width as f32, -2.0 * jitter[1] / height as f32, 0.0));
        // last frame's camera, when it is this picture's history
        let prev = d.prev.filter(|p| p.render == (width, height) && p.output == (full_w, full_h) && p.mode == mode);
        let jumped = prev.is_some_and(|p| (p.camera.position - camera.position).length() > 30.0 || p.camera.forward().dot(camera.forward()) < 0.8);
        let (vp_prev, vp_prev_here) = match prev {
            Some(p) => (p.camera.view_proj(p.aspect, p.origin), p.camera.view_proj(p.aspect, ro)),
            None => (vp_plain, vp_plain),
        };
        // last frame's model matrices: a buffer as big as the models buffer, copied into at
        // the end of each frame (a buffer: DLSS draws on DirectX 12, never on the chips that
        // read the models from a texture)
        if let Some(GpuArray::Buffer(models)) = scene.model_buf.as_ref() {
            if d.prev_models.as_ref().is_none_or(|(b, _)| b.size() < models.size()) {
                let buf = self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("dlss last models"),
                    size: models.size(),
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("dlss motion"),
                    layout: &self.dlss.pipes.as_ref().expect("dlss pipelines").motion_layout,
                    entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: d.motion_buf.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 1, resource: buf.as_entire_binding() },
                    ],
                });
                d.prev_models = Some((buf, bg));
                d.prev_epoch = u64::MAX;
            }
        }
        let models_ok = prev.is_some() && d.prev_epoch == scene.model_epoch && d.prev_models.is_some();
        let mu = MotionUniform {
            vp: vp_plain.to_cols_array_2d(),
            vp_prev: vp_prev.to_cols_array_2d(),
            vp_prev_here: vp_prev_here.to_cols_array_2d(),
            vp_inv: vp_plain.inverse().to_cols_array_2d(),
            params: [width as f32, height as f32, if models_ok { 1.0 } else { 0.0 }, d.prev_entries as f32],
        };
        self.queue.write_buffer(&d.motion_buf, 0, bytemuck::bytes_of(&mu));
        self.dlss.pending = Some(DlssFrame {
            vp: vp_plain,
            vp_prev_here,
            jitter: [sign * jitter[0], sign * jitter[1]],
            reset: prev.is_none() || jumped,
            aspect,
        });
        shift * vp_plain
    }

    /// The depth prepass of a DLSS frame: its colour targets (the motion and the bias mask,
    /// cleared), and the drawing - the sky's motion under everything, the scene's over it with
    /// the motion pipelines, then the rain films' drops. None: the ordinary prepass.
    pub(crate) fn dlss_prepass_targets(&self, f: &FrameCtx) -> Option<[Option<wgpu::RenderPassColorAttachment<'_>>; 2]> {
        let (_, t, _) = self.dlss_prepass(f)?;
        let cleared = |view| {
            Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
            })
        };
        Some([cleared(&t.motion_view), cleared(&t.bias_view)])
    }

    fn dlss_prepass(&self, f: &FrameCtx) -> Option<(&DlssPipelines, &DlssTargets, &wgpu::BindGroup)> {
        if !f.dlss_frame {
            return None;
        }
        let d = self.dlss.state.as_ref()?;
        Some((self.dlss.pipes.as_ref()?, d.targets.as_ref()?, &d.prev_models.as_ref()?.1))
    }

    /// The prepass's draws (see `dlss_prepass_targets`); false: not a DLSS frame, the
    /// ordinary prepass draws.
    pub(crate) fn encode_dlss_prepass<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, scene: &'a Scene, f: &FrameCtx, prepass_batches: &[Batch], rain_batches: &[Batch]) -> bool {
        let Some((p, _, bg)) = self.dlss_prepass(f) else { return false };
        pass.set_bind_group(3, bg, &[]);
        pass.set_pipeline(&p.sky);
        pass.draw(0..3, 0..1);
        pass.set_bind_group(0, scene.camera_bind_group.as_ref().unwrap(), &[]);
        encode_batches(pass, scene, prepass_batches, |pipe| &p.prepass[pipe as usize]);
        if !rain_batches.is_empty() {
            encode_batches(pass, scene, rain_batches, |_| &p.rain);
            encode_batches(pass, scene, rain_batches, |_| &p.rain_bias);
        }
        true
    }

    /// The end of a DLSS frame's scene commands: this frame's model matrices kept for the
    /// next one's motion vectors, and each texture DLSS reads left in the state DLSS is told
    /// (`dlss::Runtime::evaluate`): wgpu leaves a texture in the state of its last use, and
    /// these passes of nothing but their attachments are that last use - colour targets,
    /// the depth written.
    pub(crate) fn dlss_end_of_scene(&mut self, encoder: &mut wgpu::CommandEncoder, scene: &Scene) {
        let Some(d) = self.dlss.state.as_mut() else { return };
        // (the prepass read last frame's copy before this: its commands come first)
        if let (Some(GpuArray::Buffer(models)), Some((prev, _))) = (scene.model_buf.as_ref(), d.prev_models.as_ref()) {
            let bytes = (scene.uploaded_entries as u64 * 64).min(models.size()).min(prev.size());
            if bytes > 0 {
                encoder.copy_buffer_to_buffer(models, 0, prev, 0, bytes);
            }
            d.prev_epoch = scene.model_epoch;
            d.prev_entries = (bytes / 64) as u32;
        }
        let (Some(t), Some(ao)) = (d.targets.as_ref(), self.targets.ao.as_ref()) else { return };
        let keep = |view| {
            Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
            })
        };
        drop(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("dlss inputs"),
            color_attachments: &[keep(&t.color_view), keep(&t.motion_view), keep(&t.bias_view)],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &ao.depth_view,
                depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        }));
        drop(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("dlss output"),
            color_attachments: &[keep(&t.output_view)],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        }));
    }

    /// After a DLSS frame's scene submit: DLSS on wgpu's queue, then its result drawn into
    /// the window with the HUD over it. When DLSS fails, the picture is scaled up as without
    /// it, and DLSS stays off from then on.
    pub(crate) fn dlss_present(&mut self, scene: &Scene, f: &FrameCtx) {
        let Some(frame) = self.dlss.pending.take().filter(|_| f.dlss_frame) else { return };
        let (camera, ro, target, overlays) = (f.camera, f.ro, f.target, f.overlays.len());
        let (render, full) = ((f.width, f.height), (f.full_w, f.full_h));
        let mode = self.options.dlss;
        let (Some(d), Some(ao)) = (self.dlss.state.as_mut(), self.targets.ao.as_ref()) else { return };
        let Some(t) = d.targets.as_ref() else { return };
        let fov = camera.fov_deg.to_radians();
        let proj = Mat4::perspective_rh(fov, frame.aspect, camera.far, camera.near);
        let constants = dlss::FrameConstants {
            view_to_clip: sl_matrix(proj),
            clip_to_view: sl_matrix(proj.inverse()),
            clip_to_prev_clip: sl_matrix(frame.vp_prev_here * frame.vp.inverse()),
            prev_clip_to_clip: sl_matrix(frame.vp * frame.vp_prev_here.inverse()),
            jitter: frame.jitter,
            camera_pos: (camera.position - ro).as_vec3().to_array(),
            camera_up: camera.up().to_array(),
            camera_right: camera.right().to_array(),
            camera_fwd: camera.forward().to_array(),
            near: camera.near,
            far: camera.far,
            fov,
            aspect: frame.aspect,
            reset: frame.reset,
        };
        let inputs = dlss::Inputs { color: &t.color, depth: &ao.depth_tex, motion: &t.motion, bias: &t.bias, output: &t.output_tex, render_size: render, output_size: full };
        let ok = match d.runtime.evaluate(&self.device, &self.queue, mode, &inputs, &constants) {
            Ok(()) => true,
            Err(e) => {
                log::error!("DLSS failed: {e:#}; drawing without it from now on");
                false
            }
        };
        d.keep.push_back(vec![t.color.clone(), ao.depth_tex.clone(), t.motion.clone(), t.bias.clone(), t.output_tex.clone()]);
        while d.keep.len() > 4 {
            d.keep.pop_front();
        }
        d.prev = Some(DlssPrev { camera: *camera, aspect: frame.aspect, origin: ro, render, output: full, mode });
        // (2: DLSS hands back the sRGB-encoded values it read: the blit decodes them, or the
        // window's sRGB target encoded them a second time - a grey veil over everything; a
        // window of a plain format stores what it is given, so they go in as they are)
        let decode = if self.format.is_srgb() { 2.0 } else { 0.0 };
        self.queue.write_buffer(&d.blit_buf, 0, bytemuck::cast_slice(&[full.0 as f32, full.1 as f32, 0.0, decode]));
        // (without DLSS's result: the picture scaled up as the render scale does it)
        let bg = if ok { t.output_bg.clone() } else { t.color_bg.clone() };
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("dlss hud") });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("dlss hud"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.scale.upscale_pipeline);
            pass.set_bind_group(0, &bg, &[]);
            pass.draw(0..3, 0..1);
            if overlays > 0 {
                pass.set_pipeline(&self.overlay_pipeline_1x);
                for k in 0..overlays {
                    if let Some((_, _, bg, _)) = scene.overlay_res.get(k) {
                        pass.set_bind_group(0, bg, &[]);
                        pass.draw(0..6, 0..1);
                    }
                }
            }
        }
        self.queue.submit(Some(encoder.finish()));
        if !ok {
            self.dlss.failed = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dlss_jitter_is_halton() {
        assert_eq!(halton(1, 2), 0.5);
        assert_eq!(halton(2, 2), 0.25);
        assert_eq!(halton(3, 2), 0.75);
        assert!((halton(1, 3) - 1.0 / 3.0).abs() < 1e-6);
        assert!((halton(2, 3) - 2.0 / 3.0).abs() < 1e-6);
        // Streamline's rows are glam's columns: a translation sits in the last row
        let rows = sl_matrix(Mat4::from_translation(Vec3::new(1.0, 2.0, 3.0)));
        assert_eq!(rows[3], [1.0, 2.0, 3.0, 1.0]);
    }

    /// The motion passes are a module that compiles with the scene's (naga, as wgpu does).
    #[test]
    fn the_motion_passes_compile_with_the_scene_module() {
        let src = format!("{}\n{}", crate::shaders::scene_shader_text(false), include_str!("dlss_motion.wgsl"));
        let module = naga::front::wgsl::parse_str(&src).unwrap_or_else(|e| panic!("{}", e.emit_to_string(&src)));
        let info = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
            .validate(&module)
            .unwrap_or_else(|e| panic!("{e:?}"));
        for entry in ["vs_motion", "fs_motion", "fs_motion_test", "fs_motion_transmap", "fs_motion_rain", "fs_bias_rain", "vs_motion_sky", "fs_motion_sky"] {
            assert!(module.entry_points.iter().any(|e| e.name == entry), "{entry}");
        }
        // and as the HLSL wgpu's Direct3D 12 backend hands DXC (DLSS runs there only)
        let (module, info) = naga::back::pipeline_constants::process_overrides(&module, &info, None, &Default::default()).unwrap();
        let options = naga::back::hlsl::Options { shader_model: naga::back::hlsl::ShaderModel::V6_5, ..Default::default() };
        let mut out = String::new();
        let pipeline = Default::default();
        naga::back::hlsl::Writer::new(&mut out, &options, &pipeline).write(&module, &info, None).unwrap_or_else(|e| panic!("{e:?}"));
        assert!(out.contains("vs_motion") && out.contains("fs_bias_rain"));
    }
}
