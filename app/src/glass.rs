//! Liquid glass for the controls: rounded panes that bend the picture behind
//! them in at the rim, frost it, split its colors a little where it bends
//! most and catch the light on their edges. Drawn by our own shader
//! (`glass.wgsl`), reading the window's background, which `background.rs`
//! also draws into a texture of its own each frame.
//!
//! What is behind is only that background: the cover, titles and lyrics are
//! not in it. Where something else fills the window (a lyric style of its
//! own), [`set_flat`] says what color the glass stands on instead.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};

use eframe::egui::{self, Color32, Painter, Rect, Shape};
use eframe::egui_wgpu::{self, CallbackResources, CallbackTrait, ScreenDescriptor, wgpu};

/// Panes one window can draw per frame; more are left out.
const MAX_PANES: u32 = 256;
/// Between the panes' uniforms in the buffer (wgpu's minimum alignment).
const STRIDE: u64 = 256;
/// The backdrop is drawn at this fraction of the window's size.
pub const BACKDROP_SCALE: f32 = 0.5;

/// How a pane looks. Lengths are in points.
#[derive(Clone, Copy, Debug)]
pub struct Look {
    /// Laid over what's behind; its alpha is how much.
    pub tint: Color32,
    /// How far the rim pulls the picture in.
    pub bend: f32,
    /// Blur radius.
    pub frost: f32,
    /// How much the colors split where it bends (0..1).
    pub dispersion: f32,
    /// How bright the rim's light is (0..1).
    pub rim: f32,
    /// Brightens what's behind (0 = as is).
    pub lift: f32,
    /// A soft shadow under the pane (0 = none).
    pub shadow: f32,
}

impl Look {
    /// The big panels: frosted deep, dark enough to read on.
    pub const PANEL: Self = Self {
        tint: Color32::from_rgba_premultiplied(10, 10, 16, 150),
        bend: 26.0,
        frost: 22.0,
        dispersion: 0.5,
        rim: 0.22,
        lift: -0.1,
        shadow: 1.0,
    };
    /// Pills and bars over the picture: clearer, so it shows through.
    pub const PILL: Self = Self {
        tint: Color32::from_rgba_premultiplied(8, 8, 14, 95),
        bend: 16.0,
        frost: 7.0,
        dispersion: 0.6,
        rim: 0.3,
        lift: 0.0,
        shadow: 0.6,
    };
    /// Small things that read best light, like a knob or a picked tab.
    pub const BEAD: Self = Self {
        tint: Color32::from_rgba_premultiplied(255, 255, 255, 40),
        bend: 9.0,
        frost: 2.0,
        dispersion: 0.8,
        rim: 0.5,
        lift: 0.15,
        shadow: 0.4,
    };

    pub fn tint(self, tint: Color32) -> Self {
        Self { tint, ..self }
    }
}

/// Draws a glass pane over `rect` with corners of `radius`.
pub fn pane(painter: &Painter, rect: Rect, radius: f32, look: Look) {
    for shape in shapes(painter, rect, radius, look) {
        painter.add(shape);
    }
}

/// The shapes of [`pane`] (its shadow and the glass), at `painter`'s
/// opacity, for putting under something already drawn.
pub fn shapes(painter: &Painter, rect: Rect, radius: f32, look: Look) -> [Shape; 2] {
    let opacity = painter.opacity();
    if opacity <= 0.0 || !rect.is_positive() {
        return [Shape::Noop, Shape::Noop];
    }
    let shadow = if look.shadow > 0.0 {
        egui::Shadow {
            offset: [0, (rect.height() * 0.08).clamp(2.0, 24.0) as i8],
            blur: (rect.height() * 0.35).clamp(6.0, 80.0) as u8,
            spread: 0,
            color: Color32::from_black_alpha((90.0 * look.shadow * opacity) as u8),
        }
        .as_shape(rect, radius)
        .into()
    } else {
        Shape::Noop
    };
    let ctx = painter.ctx();
    let key = ctx.viewport_id().0.value();
    let flat = ctx.data(|d| d.get_temp::<Color32>(flat_id(key)));
    let half = (rect.width().min(rect.height()) / 2.0).max(1.0);
    let rgba = |c: Color32| {
        let [r, g, b, a] = c.to_srgba_unmultiplied();
        [r, g, b, a].map(|v| f32::from(v) / 255.0)
    };
    let glass = egui_wgpu::Callback::new_paint_callback(
        rect,
        Pane {
            key,
            frame: ctx.cumulative_pass_nr(),
            look,
            radius: (radius / half).clamp(0.0, 1.0),
            opacity,
            flat: flat.map_or([0.0; 4], rgba),
            tint: rgba(look.tint),
            slot: AtomicU32::new(u32::MAX),
        },
    );
    [shadow, glass.into()]
}

/// Until the next call, the glass in this window stands on `color` instead
/// of the background (`None`: on the background again).
pub fn set_flat(ctx: &egui::Context, color: Option<Color32>) {
    let id = flat_id(ctx.viewport_id().0.value());
    ctx.data_mut(|d| match color {
        Some(c) => {
            d.insert_temp(id, c);
        }
        None => d.remove::<Color32>(id),
    });
}

fn flat_id(key: u64) -> egui::Id {
    egui::Id::new(("ozbeat-glass-flat", key))
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    tint: [f32; 4],
    look: [f32; 4],
    shape: [f32; 4],
    screen: [f32; 4],
    flat: [f32; 4],
}

struct Pane {
    /// The window (viewport) it is in, and the frame.
    key: u64,
    frame: u64,
    look: Look,
    radius: f32,
    opacity: f32,
    flat: [f32; 4],
    tint: [f32; 4],
    /// Where its uniforms went in the buffer; set in `prepare`.
    slot: AtomicU32,
}

/// The background as one window's glass sees it.
struct Backdrop {
    size: [u32; 2],
    /// The frame it was last drawn in.
    frame: u64,
    view: wgpu::TextureView,
    bind_group: wgpu::BindGroup,
}

pub struct Resources {
    pipeline: wgpu::RenderPipeline,
    backdrop_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    format: wgpu::TextureFormat,
    srgb_target: bool,
    uniforms: wgpu::Buffer,
    pane_bind_group: wgpu::BindGroup,
    /// For windows without a backdrop yet.
    empty: wgpu::BindGroup,
    backdrops: HashMap<u64, Backdrop>,
    /// The window and frame the slots are being handed out for, and the next.
    filling: (u64, u64),
    next: u32,
}

impl Resources {
    /// The texture `key`'s background goes into this frame, sized for a
    /// window of `screen` pixels.
    pub fn backdrop_target(
        &mut self,
        device: &wgpu::Device,
        key: u64,
        frame: u64,
        screen: [u32; 2],
    ) -> wgpu::TextureView {
        let size = screen.map(|v| ((v as f32 * BACKDROP_SCALE) as u32).max(1));
        if self.backdrops.get(&key).is_none_or(|b| b.size != size) {
            let view = texture(device, self.format, size);
            let bind_group = backdrop_bind_group(device, &self.backdrop_layout, &view, &self.sampler);
            self.backdrops.insert(key, Backdrop { size, frame, view, bind_group });
        }
        let backdrop = self.backdrops.get_mut(&key).unwrap();
        backdrop.frame = frame;
        backdrop.view.clone()
    }

    /// `key`'s backdrop, if it was drawn in `frame`.
    fn backdrop(&self, key: u64, frame: u64) -> Option<&Backdrop> {
        self.backdrops.get(&key).filter(|b| b.frame == frame)
    }
}

fn texture(device: &wgpu::Device, format: wgpu::TextureFormat, size: [u32; 2]) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("glass backdrop"),
            size: wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        })
        .create_view(&wgpu::TextureViewDescriptor::default())
}

fn backdrop_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    view: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("glass backdrop"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    })
}

/// Creates the pipeline once and stores it in egui's renderer.
pub fn init(render_state: &egui_wgpu::RenderState) {
    let device = &render_state.device;
    let format = render_state.target_format;
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("glass"),
        source: wgpu::ShaderSource::Wgsl(include_str!("glass.wgsl").into()),
    });
    let backdrop_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("glass backdrop"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    });
    let pane_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("glass pane"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: true,
                min_binding_size: wgpu::BufferSize::new(size_of::<Uniforms>() as u64),
            },
            count: None,
        }],
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("glass"),
        bind_group_layouts: &[Some(&backdrop_layout), Some(&pane_layout)],
        immediate_size: 0,
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("glass"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    });
    let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("glass panes"),
        size: STRIDE * u64::from(MAX_PANES),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let pane_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("glass panes"),
        layout: &pane_layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                buffer: &uniforms,
                offset: 0,
                size: wgpu::BufferSize::new(size_of::<Uniforms>() as u64),
            }),
        }],
    });
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("glass backdrop"),
        address_mode_u: wgpu::AddressMode::MirrorRepeat,
        address_mode_v: wgpu::AddressMode::MirrorRepeat,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    let empty_view = texture(device, format, [1, 1]);
    let empty = backdrop_bind_group(device, &backdrop_layout, &empty_view, &sampler);
    render_state
        .renderer
        .write()
        .callback_resources
        .insert(Resources {
            pipeline,
            backdrop_layout,
            sampler,
            format,
            srgb_target: format.is_srgb(),
            uniforms,
            pane_bind_group,
            empty,
            backdrops: HashMap::new(),
            filling: (0, u64::MAX),
            next: 0,
        });
}

impl CallbackTrait for Pane {
    fn prepare(
        &self,
        _device: &wgpu::Device,
        queue: &wgpu::Queue,
        screen: &ScreenDescriptor,
        _encoder: &mut wgpu::CommandEncoder,
        resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let Some(res) = resources.get_mut::<Resources>() else {
            return Vec::new();
        };
        if res.filling != (self.key, self.frame) {
            res.filling = (self.key, self.frame);
            res.next = 0;
        }
        if res.next >= MAX_PANES {
            return Vec::new();
        }
        let slot = res.next;
        res.next += 1;
        self.slot.store(slot, Ordering::Relaxed);

        let ppp = screen.pixels_per_point;
        let look = self.look;
        let uniforms = Uniforms {
            tint: self.tint,
            look: [look.bend * ppp, look.frost * ppp, look.dispersion, look.rim],
            shape: [
                self.radius,
                self.opacity,
                if res.srgb_target { 1.0 } else { 0.0 },
                look.lift,
            ],
            screen: [
                screen.size_in_pixels[0] as f32,
                screen.size_in_pixels[1] as f32,
                if res.backdrop(self.key, self.frame).is_some() { 1.0 } else { 0.0 },
                0.0,
            ],
            flat: self.flat,
        };
        queue.write_buffer(&res.uniforms, u64::from(slot) * STRIDE, bytemuck::bytes_of(&uniforms));
        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        resources: &CallbackResources,
    ) {
        let Some(res) = resources.get::<Resources>() else {
            return;
        };
        let slot = self.slot.load(Ordering::Relaxed);
        if slot == u32::MAX {
            return;
        }
        let backdrop = res.backdrop(self.key, self.frame).map_or(&res.empty, |b| &b.bind_group);
        render_pass.set_pipeline(&res.pipeline);
        render_pass.set_bind_group(0, backdrop, &[]);
        render_pass.set_bind_group(1, &res.pane_bind_group, &[(u64::from(slot) * STRIDE) as u32]);
        render_pass.draw(0..3, 0..1);
    }
}
