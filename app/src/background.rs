//! The animated background, drawn by our own WGSL shader inside egui's frame.

use std::sync::Arc;

use eframe::egui_wgpu::{self, CallbackResources, CallbackTrait, ScreenDescriptor, wgpu};
use wgpu::util::DeviceExt as _;

use crate::art::Picture;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Uniforms {
    pub resolution: [f32; 2],
    pub time: f32,
    pub pulse: f32,
    pub vivid: [f32; 4],
    pub mid: [f32; 4],
    pub dark: [f32; 4],
    pub art_size: [f32; 2],
    pub has_art: f32,
    pub srgb_target: f32,
    pub prev_size: [f32; 2],
    /// How visible the previous artwork is before it fades (0 = not at all).
    pub prev_has_art: f32,
    /// 0 -> 1 while the previous artwork gives way to the current one.
    pub fade: f32,
}

/// One frame's worth of background: parameters plus the artwork to show.
pub struct Background {
    pub uniforms: Uniforms,
    /// Art id + blurred picture; uploaded to the GPU only when the id changes.
    pub art: Option<(u64, Arc<Picture>)>,
    /// The artwork fading out.
    pub prev: Option<(u64, Arc<Picture>)>,
}

struct Resources {
    pipeline: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    bind_group: wgpu::BindGroup,
    /// Uploaded artwork by art id: the current and the previous one.
    textures: Vec<(u64, wgpu::TextureView)>,
    placeholder: wgpu::TextureView,
    /// Art ids (current, previous) in `bind_group`.
    bound: (Option<u64>, Option<u64>),
    srgb_target: bool,
}

/// Creates the pipeline once and stores it in egui's renderer.
pub fn init(render_state: &egui_wgpu::RenderState) {
    let device = &render_state.device;
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("background"),
        source: wgpu::ShaderSource::Wgsl(include_str!("background.wgsl").into()),
    });

    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("background"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 3,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
        ],
    });

    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("background"),
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });

    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("background"),
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
                format: render_state.target_format,
                blend: Some(wgpu::BlendState::REPLACE),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    });

    let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("background uniforms"),
        size: size_of::<Uniforms>() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    // Mirrored edges: the warp may push sampling slightly outside the image.
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("background art"),
        address_mode_u: wgpu::AddressMode::MirrorRepeat,
        address_mode_v: wgpu::AddressMode::MirrorRepeat,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });

    let placeholder = upload(
        device,
        &render_state.queue,
        &Picture {
            size: [1, 1],
            rgba: vec![0, 0, 0, 255],
        },
    );
    let bind_group = art_bind_group(
        device,
        &layout,
        &uniforms,
        &sampler,
        [&placeholder, &placeholder],
    );

    render_state
        .renderer
        .write()
        .callback_resources
        .insert(Resources {
            pipeline,
            uniforms,
            layout,
            sampler,
            bind_group,
            textures: Vec::new(),
            placeholder,
            bound: (None, None),
            srgb_target: render_state.target_format.is_srgb(),
        });
}

fn upload(device: &wgpu::Device, queue: &wgpu::Queue, picture: &Picture) -> wgpu::TextureView {
    // Non-sRGB format on purpose: the shader works in sRGB-encoded values.
    let texture = device.create_texture_with_data(
        queue,
        &wgpu::TextureDescriptor {
            label: Some("background art"),
            size: wgpu::Extent3d {
                width: picture.size[0] as u32,
                height: picture.size[1] as u32,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        },
        wgpu::util::TextureDataOrder::LayerMajor,
        &picture.rgba,
    );
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

/// Binds the current (`art[0]`) and the previous (`art[1]`) artwork.
fn art_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    uniforms: &wgpu::Buffer,
    sampler: &wgpu::Sampler,
    art: [&wgpu::TextureView; 2],
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("background"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(art[0]),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::TextureView(art[1]),
            },
        ],
    })
}

impl CallbackTrait for Background {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        screen: &ScreenDescriptor,
        _encoder: &mut wgpu::CommandEncoder,
        resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let Some(res) = resources.get_mut::<Resources>() else {
            return Vec::new();
        };

        let id = |art: &Option<(u64, Arc<Picture>)>| art.as_ref().map(|(id, _)| *id);
        let wanted = (id(&self.art), id(&self.prev));
        if wanted != res.bound {
            for (id, picture) in [&self.art, &self.prev].into_iter().flatten() {
                if !res.textures.iter().any(|(t, _)| t == id) {
                    res.textures.push((*id, upload(device, queue, picture)));
                }
            }
            res.textures
                .retain(|(t, _)| Some(*t) == wanted.0 || Some(*t) == wanted.1);
            let view = |id: Option<u64>| {
                res.textures
                    .iter()
                    .find(|(t, _)| Some(*t) == id)
                    .map_or(&res.placeholder, |(_, v)| v)
            };
            res.bind_group = art_bind_group(
                device,
                &res.layout,
                &res.uniforms,
                &res.sampler,
                [view(wanted.0), view(wanted.1)],
            );
            res.bound = wanted;
        }

        let mut uniforms = self.uniforms;
        uniforms.resolution = screen.size_in_pixels.map(|v| v as f32);
        uniforms.srgb_target = if res.srgb_target { 1.0 } else { 0.0 };
        queue.write_buffer(&res.uniforms, 0, bytemuck::bytes_of(&uniforms));
        Vec::new()
    }

    fn paint(
        &self,
        _info: eframe::egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        resources: &CallbackResources,
    ) {
        let Some(res) = resources.get::<Resources>() else {
            return;
        };
        render_pass.set_pipeline(&res.pipeline);
        render_pass.set_bind_group(0, &res.bind_group, &[]);
        render_pass.draw(0..3, 0..1);
    }
}
