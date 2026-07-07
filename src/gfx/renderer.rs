//! The HD-2D renderer. Lives inside egui's wgpu callback resources; the UI
//! hands it a `FrameInput` each frame and it renders the scene offscreen in
//! HDR, runs the post chain (blur → bloom/tilt-shift), and composites into
//! the egui render pass.

use std::sync::Arc;

use eframe::egui_wgpu::{self, wgpu};
use glam::{Mat4, Vec3};

use crate::core::data::MapData;
use crate::gfx::mesh::{build_terrain_mesh, SpriteInstance, TerrainVertex};
use crate::gfx::pixelart::{Atlas, ATLAS_H, ATLAS_W};

pub const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
pub const MAX_LIGHTS: usize = 32;

#[derive(Clone, Copy, Debug)]
pub struct LightSpec {
    pub pos: Vec3,
    pub radius: f32,
    pub color: [f32; 3],
}

#[derive(Clone, Copy, Debug)]
pub struct PostSettings {
    pub bloom_strength: f32,
    pub bloom_threshold: f32,
    /// Vertical screen position of the tilt-shift focus band (0 = top).
    pub focus_y: f32,
    pub dof_strength: f32,
    pub vignette: f32,
    pub exposure: f32,
    pub saturation: f32,
}

impl Default for PostSettings {
    fn default() -> Self {
        PostSettings {
            bloom_strength: 0.6,
            bloom_threshold: 0.75,
            focus_y: 0.55,
            dof_strength: 0.85,
            vignette: 0.35,
            exposure: 1.15,
            saturation: 1.12,
        }
    }
}

/// Everything the UI thread hands the renderer for one frame.
pub struct FrameInput {
    pub view_proj: Mat4,
    pub cam_pos: Vec3,
    pub cam_right: Vec3,
    pub sun_dir: Vec3,
    pub sun_color: [f32; 3],
    pub ambient: [f32; 3],
    pub fog_color: [f32; 3],
    pub fog_density: f32,
    pub darkness: f32,
    pub time: f32,
    pub lights: Vec<LightSpec>,
    pub sprites_cutout: Vec<SpriteInstance>,
    pub sprites_blend: Vec<SpriteInstance>,
    pub map: Arc<MapData>,
    /// Bump to force a terrain rebuild (map id in the high bits keeps maps distinct).
    pub map_revision: u64,
    pub post: PostSettings,
    pub viewport_px: [u32; 2],
}

pub struct Hd2dCallback {
    pub input: Arc<FrameInput>,
}

// ---------------------------------------------------------------------------
// Raw uniform layouts (must match shaders.wgsl)
// ---------------------------------------------------------------------------

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct LightRaw {
    pos_radius: [f32; 4],
    color: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct SceneUniforms {
    view_proj: [[f32; 4]; 4],
    cam_pos: [f32; 4],
    cam_right: [f32; 4],
    sun_dir: [f32; 4],
    sun_color: [f32; 4],
    ambient: [f32; 4],
    fog_color_density: [f32; 4],
    misc: [f32; 4],
    light_count: [u32; 4],
    lights: [LightRaw; MAX_LIGHTS],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct PostUniforms {
    a: [f32; 4],
    b: [f32; 4],
    c: [f32; 4],
}

// ---------------------------------------------------------------------------
// Renderer
// ---------------------------------------------------------------------------

struct Targets {
    size: [u32; 2],
    hdr_view: wgpu::TextureView,
    depth_view: wgpu::TextureView,
    half_a_view: wgpu::TextureView,
    half_b_view: wgpu::TextureView,
    // Post bind groups (built against the views above).
    bg_downsample: wgpu::BindGroup, // src = hdr
    bg_blur_h1: wgpu::BindGroup,    // src = half_a
    bg_blur_v1: wgpu::BindGroup,    // src = half_b
    bg_blur_h2: wgpu::BindGroup,
    bg_blur_v2: wgpu::BindGroup,
    bg_composite: wgpu::BindGroup, // src = hdr, aux = half_a
}

pub struct Hd2dRenderer {
    atlas: Arc<Atlas>,
    scene_ub: wgpu::Buffer,
    post_composite_ub: wgpu::Buffer,
    post_blur_ubs: [wgpu::Buffer; 5], // downsample, h1, v1, h2, v2
    scene_bg: wgpu::BindGroup,
    post_bgl: wgpu::BindGroupLayout,
    linear_sampler: wgpu::Sampler,
    pipe_terrain: wgpu::RenderPipeline,
    pipe_sprite_cutout: wgpu::RenderPipeline,
    pipe_sprite_blend: wgpu::RenderPipeline,
    pipe_downsample: wgpu::RenderPipeline,
    pipe_blur_h: wgpu::RenderPipeline,
    pipe_blur_v: wgpu::RenderPipeline,
    pipe_composite: wgpu::RenderPipeline,
    target_is_srgb: bool,
    targets: Option<Targets>,
    terrain_vbuf: Option<wgpu::Buffer>,
    terrain_ibuf: Option<wgpu::Buffer>,
    terrain_index_count: u32,
    terrain_revision: u64,
    inst_cutout: GrowBuffer,
    inst_blend: GrowBuffer,
}

/// A vertex/instance buffer that grows as needed.
struct GrowBuffer {
    buf: Option<wgpu::Buffer>,
    capacity: u64,
    label: &'static str,
}

impl GrowBuffer {
    fn new(label: &'static str) -> Self {
        GrowBuffer {
            buf: None,
            capacity: 0,
            label,
        }
    }
    fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, data: &[u8]) {
        let needed = data.len() as u64;
        if needed == 0 {
            return;
        }
        if self.buf.is_none() || self.capacity < needed {
            self.capacity = needed.next_power_of_two().max(4096);
            self.buf = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(self.label),
                size: self.capacity,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
        queue.write_buffer(self.buf.as_ref().unwrap(), 0, data);
    }
}

impl Hd2dRenderer {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target_format: wgpu::TextureFormat,
        atlas: Arc<Atlas>,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("nom-shaders"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders.wgsl").into()),
        });

        // Atlas texture.
        let atlas_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("nom-atlas"),
            size: wgpu::Extent3d {
                width: ATLAS_W,
                height: ATLAS_H,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &atlas_tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &atlas.pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(ATLAS_W * 4),
                rows_per_image: Some(ATLAS_H),
            },
            wgpu::Extent3d {
                width: ATLAS_W,
                height: ATLAS_H,
                depth_or_array_layers: 1,
            },
        );
        let atlas_view = atlas_tex.create_view(&wgpu::TextureViewDescriptor::default());

        let nearest_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("nom-nearest"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        let linear_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("nom-linear"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });

        // Scene bind group layout: uniforms + atlas + sampler.
        let scene_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("nom-scene-bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
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
            ],
        });

        let scene_ub = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("nom-scene-ub"),
            size: std::mem::size_of::<SceneUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let scene_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("nom-scene-bg"),
            layout: &scene_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: scene_ub.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&atlas_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&nearest_sampler),
                },
            ],
        });

        // Post bind group layout: uniforms + src texture + sampler + aux texture.
        let post_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("nom-post-bgl"),
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

        let mk_post_ub = |label: &str| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: std::mem::size_of::<PostUniforms>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let post_composite_ub = mk_post_ub("nom-post-composite-ub");
        let post_blur_ubs = [
            mk_post_ub("nom-post-down-ub"),
            mk_post_ub("nom-post-h1-ub"),
            mk_post_ub("nom-post-v1-ub"),
            mk_post_ub("nom-post-h2-ub"),
            mk_post_ub("nom-post-v2-ub"),
        ];

        // Pipelines.
        let scene_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("nom-scene-pl"),
            bind_group_layouts: &[Some(&scene_bgl)],
            immediate_size: 0,
        });
        let post_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("nom-post-pl"),
            bind_group_layouts: &[Some(&post_bgl)],
            immediate_size: 0,
        });

        let terrain_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<TerrainVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2, 3 => Uint32],
        };
        let sprite_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<SpriteInstance>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2, 2 => Float32x2, 3 => Float32x2, 4 => Float32x4, 5 => Uint32],
        };

        let depth_write = |write: bool| wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(write),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        };
        let hdr_target = |blend: Option<wgpu::BlendState>| {
            Some(wgpu::ColorTargetState {
                format: HDR_FORMAT,
                blend,
                write_mask: wgpu::ColorWrites::ALL,
            })
        };

        let pipe_terrain = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("nom-terrain"),
            layout: Some(&scene_pl),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_terrain"),
                compilation_options: Default::default(),
                buffers: &[terrain_layout],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_terrain"),
                compilation_options: Default::default(),
                targets: &[hdr_target(None)],
            }),
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(depth_write(true)),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let pipe_sprite_cutout = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("nom-sprite-cutout"),
            layout: Some(&scene_pl),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_sprite"),
                compilation_options: Default::default(),
                buffers: &[sprite_layout.clone()],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_sprite_cutout"),
                compilation_options: Default::default(),
                targets: &[hdr_target(None)],
            }),
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(depth_write(true)),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let pipe_sprite_blend = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("nom-sprite-blend"),
            layout: Some(&scene_pl),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_sprite"),
                compilation_options: Default::default(),
                buffers: &[sprite_layout],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_sprite_blend"),
                compilation_options: Default::default(),
                targets: &[hdr_target(Some(wgpu::BlendState::ALPHA_BLENDING))],
            }),
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(depth_write(false)),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let mk_post_pipe = |label: &str, entry: &str, format: wgpu::TextureFormat| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&post_pl),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_fullscreen"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };

        let pipe_downsample = mk_post_pipe("nom-downsample", "fs_downsample", HDR_FORMAT);
        let pipe_blur_h = mk_post_pipe("nom-blur-h", "fs_blur_h", HDR_FORMAT);
        let pipe_blur_v = mk_post_pipe("nom-blur-v", "fs_blur_v", HDR_FORMAT);
        let pipe_composite = mk_post_pipe("nom-composite", "fs_composite", target_format);

        Hd2dRenderer {
            atlas,
            scene_ub,
            post_composite_ub,
            post_blur_ubs,
            scene_bg,
            post_bgl,
            linear_sampler,
            pipe_terrain,
            pipe_sprite_cutout,
            pipe_sprite_blend,
            pipe_downsample,
            pipe_blur_h,
            pipe_blur_v,
            pipe_composite,
            target_is_srgb: target_format.is_srgb(),
            targets: None,
            terrain_vbuf: None,
            terrain_ibuf: None,
            terrain_index_count: 0,
            terrain_revision: u64::MAX,
            inst_cutout: GrowBuffer::new("nom-inst-cutout"),
            inst_blend: GrowBuffer::new("nom-inst-blend"),
        }
    }

    fn ensure_targets(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, size: [u32; 2]) {
        let size = [size[0].max(8), size[1].max(8)];
        if let Some(t) = &self.targets {
            if t.size == size {
                return;
            }
        }
        let mk_tex = |label: &str, w: u32, h: u32, format: wgpu::TextureFormat| {
            let tex = device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            tex.create_view(&wgpu::TextureViewDescriptor::default())
        };
        let (w, h) = (size[0], size[1]);
        let (hw, hh) = ((w / 2).max(4), (h / 2).max(4));
        let hdr_view = mk_tex("nom-hdr", w, h, HDR_FORMAT);
        let depth_view = mk_tex("nom-depth", w, h, DEPTH_FORMAT);
        let half_a_view = mk_tex("nom-half-a", hw, hh, HDR_FORMAT);
        let half_b_view = mk_tex("nom-half-b", hw, hh, HDR_FORMAT);

        // Static blur uniforms for this size (texel size + radius scale).
        let texel = [1.0 / hw as f32, 1.0 / hh as f32];
        let blur_params: [(usize, f32); 5] = [(0, 1.0), (1, 1.0), (2, 1.0), (3, 2.2), (4, 2.2)];
        for (i, radius) in blur_params {
            let u = PostUniforms {
                a: [0.0; 4],
                b: [0.0; 4],
                c: [texel[0], texel[1], radius, 0.0],
            };
            queue.write_buffer(&self.post_blur_ubs[i], 0, bytemuck::bytes_of(&u));
        }

        let mk_bg =
            |label: &str, ub: &wgpu::Buffer, src: &wgpu::TextureView, aux: &wgpu::TextureView| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some(label),
                    layout: &self.post_bgl,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: ub.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(src),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::Sampler(&self.linear_sampler),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: wgpu::BindingResource::TextureView(aux),
                        },
                    ],
                })
            };

        // The aux slot is unused outside the composite pass; bind the HDR view
        // (never a post-pass render target) so usages don't conflict.
        let bg_downsample = mk_bg("nom-bg-down", &self.post_blur_ubs[0], &hdr_view, &hdr_view);
        let bg_blur_h1 = mk_bg("nom-bg-h1", &self.post_blur_ubs[1], &half_a_view, &hdr_view);
        let bg_blur_v1 = mk_bg("nom-bg-v1", &self.post_blur_ubs[2], &half_b_view, &hdr_view);
        let bg_blur_h2 = mk_bg("nom-bg-h2", &self.post_blur_ubs[3], &half_a_view, &hdr_view);
        let bg_blur_v2 = mk_bg("nom-bg-v2", &self.post_blur_ubs[4], &half_b_view, &hdr_view);
        let bg_composite = mk_bg(
            "nom-bg-composite",
            &self.post_composite_ub,
            &hdr_view,
            &half_a_view,
        );

        self.targets = Some(Targets {
            size,
            hdr_view,
            depth_view,
            half_a_view,
            half_b_view,
            bg_downsample,
            bg_blur_h1,
            bg_blur_v1,
            bg_blur_h2,
            bg_blur_v2,
            bg_composite,
        });
    }

    fn ensure_terrain(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, input: &FrameInput) {
        if self.terrain_revision == input.map_revision && self.terrain_vbuf.is_some() {
            return;
        }
        let (verts, indices) = build_terrain_mesh(&input.map, &self.atlas);
        let vbytes: &[u8] = bytemuck::cast_slice(&verts);
        let ibytes: &[u8] = bytemuck::cast_slice(&indices);
        let need_v = vbytes.len() as u64;
        let need_i = ibytes.len() as u64;
        let cap_ok =
            |b: &Option<wgpu::Buffer>, need: u64| b.as_ref().is_some_and(|b| b.size() >= need);
        if !cap_ok(&self.terrain_vbuf, need_v) {
            self.terrain_vbuf = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("nom-terrain-v"),
                size: need_v.next_power_of_two(),
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
        if !cap_ok(&self.terrain_ibuf, need_i) {
            self.terrain_ibuf = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("nom-terrain-i"),
                size: need_i.next_power_of_two(),
                usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
        queue.write_buffer(self.terrain_vbuf.as_ref().unwrap(), 0, vbytes);
        queue.write_buffer(self.terrain_ibuf.as_ref().unwrap(), 0, ibytes);
        self.terrain_index_count = indices.len() as u32;
        self.terrain_revision = input.map_revision;
    }

    fn write_uniforms(&self, queue: &wgpu::Queue, input: &FrameInput) {
        let mut lights = [LightRaw {
            pos_radius: [0.0; 4],
            color: [0.0; 4],
        }; MAX_LIGHTS];
        let count = input.lights.len().min(MAX_LIGHTS);
        for (i, l) in input.lights.iter().take(MAX_LIGHTS).enumerate() {
            lights[i] = LightRaw {
                pos_radius: [l.pos.x, l.pos.y, l.pos.z, l.radius],
                color: [l.color[0], l.color[1], l.color[2], 0.0],
            };
        }
        let u = SceneUniforms {
            view_proj: input.view_proj.to_cols_array_2d(),
            cam_pos: [input.cam_pos.x, input.cam_pos.y, input.cam_pos.z, 0.0],
            cam_right: [input.cam_right.x, input.cam_right.y, input.cam_right.z, 0.0],
            sun_dir: [input.sun_dir.x, input.sun_dir.y, input.sun_dir.z, 0.0],
            sun_color: [
                input.sun_color[0],
                input.sun_color[1],
                input.sun_color[2],
                0.0,
            ],
            ambient: [input.ambient[0], input.ambient[1], input.ambient[2], 0.0],
            fog_color_density: [
                input.fog_color[0],
                input.fog_color[1],
                input.fog_color[2],
                input.fog_density,
            ],
            misc: [input.time, input.darkness, 0.0, 0.0],
            light_count: [count as u32, 0, 0, 0],
            lights,
        };
        queue.write_buffer(&self.scene_ub, 0, bytemuck::bytes_of(&u));

        let p = input.post;
        let comp = PostUniforms {
            a: [
                p.bloom_strength,
                p.bloom_threshold,
                p.focus_y,
                p.dof_strength,
            ],
            b: [
                p.vignette,
                p.exposure,
                p.saturation,
                if self.target_is_srgb { 1.0 } else { 2.0 },
            ],
            c: [0.0, 0.0, 0.0, input.time],
        };
        queue.write_buffer(&self.post_composite_ub, 0, bytemuck::bytes_of(&comp));
    }

    fn encode(&mut self, encoder: &mut wgpu::CommandEncoder, input: &FrameInput) {
        let Some(t) = &self.targets else { return };

        // --- Scene pass ---
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("nom-scene"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &t.hdr_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: input.fog_color[0] as f64,
                            g: input.fog_color[1] as f64,
                            b: input.fog_color[2] as f64,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &t.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &self.scene_bg, &[]);
            if self.terrain_index_count > 0 {
                pass.set_pipeline(&self.pipe_terrain);
                pass.set_vertex_buffer(0, self.terrain_vbuf.as_ref().unwrap().slice(..));
                pass.set_index_buffer(
                    self.terrain_ibuf.as_ref().unwrap().slice(..),
                    wgpu::IndexFormat::Uint32,
                );
                pass.draw_indexed(0..self.terrain_index_count, 0, 0..1);
            }
            if !input.sprites_cutout.is_empty() {
                if let Some(buf) = &self.inst_cutout.buf {
                    pass.set_pipeline(&self.pipe_sprite_cutout);
                    pass.set_vertex_buffer(0, buf.slice(..));
                    pass.draw(0..6, 0..input.sprites_cutout.len() as u32);
                }
            }
            if !input.sprites_blend.is_empty() {
                if let Some(buf) = &self.inst_blend.buf {
                    pass.set_pipeline(&self.pipe_sprite_blend);
                    pass.set_vertex_buffer(0, buf.slice(..));
                    pass.draw(0..6, 0..input.sprites_blend.len() as u32);
                }
            }
        }

        // --- Post chain: downsample + two blur iterations ---
        let post_pass = |encoder: &mut wgpu::CommandEncoder,
                         target: &wgpu::TextureView,
                         pipe: &wgpu::RenderPipeline,
                         bg: &wgpu::BindGroup| {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("nom-post"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(pipe);
            pass.set_bind_group(0, bg, &[]);
            pass.draw(0..3, 0..1);
        };

        post_pass(
            encoder,
            &t.half_a_view,
            &self.pipe_downsample,
            &t.bg_downsample,
        );
        post_pass(encoder, &t.half_b_view, &self.pipe_blur_h, &t.bg_blur_h1);
        post_pass(encoder, &t.half_a_view, &self.pipe_blur_v, &t.bg_blur_v1);
        post_pass(encoder, &t.half_b_view, &self.pipe_blur_h, &t.bg_blur_h2);
        post_pass(encoder, &t.half_a_view, &self.pipe_blur_v, &t.bg_blur_v2);
    }
}

impl egui_wgpu::CallbackTrait for Hd2dCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen_descriptor: &egui_wgpu::ScreenDescriptor,
        egui_encoder: &mut wgpu::CommandEncoder,
        callback_resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let Some(renderer) = callback_resources.get_mut::<Hd2dRenderer>() else {
            return Vec::new();
        };
        let input = &self.input;
        renderer.ensure_targets(device, queue, input.viewport_px);
        renderer.ensure_terrain(device, queue, input);
        renderer.write_uniforms(queue, input);
        renderer
            .inst_cutout
            .upload(device, queue, bytemuck::cast_slice(&input.sprites_cutout));
        renderer
            .inst_blend
            .upload(device, queue, bytemuck::cast_slice(&input.sprites_blend));
        renderer.encode(egui_encoder, input);
        Vec::new()
    }

    fn paint(
        &self,
        _info: eframe::egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        callback_resources: &egui_wgpu::CallbackResources,
    ) {
        let Some(renderer) = callback_resources.get::<Hd2dRenderer>() else {
            return;
        };
        let Some(t) = &renderer.targets else { return };
        render_pass.set_pipeline(&renderer.pipe_composite);
        render_pass.set_bind_group(0, &t.bg_composite, &[]);
        render_pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Headless GPU smoke test: shader must parse and every pipeline must build.
    #[test]
    fn shader_and_pipelines_build() {
        let instance = wgpu::Instance::default();
        let Ok(adapter) =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::LowPower,
                force_fallback_adapter: false,
                compatible_surface: None,
            }))
        else {
            eprintln!("no GPU adapter available; skipping");
            return;
        };
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
        device.on_uncaptured_error(Arc::new(|e: wgpu::Error| panic!("wgpu error: {e}")));
        let atlas = Arc::new(crate::gfx::pixelart::build_atlas());
        let mut r = Hd2dRenderer::new(&device, &queue, wgpu::TextureFormat::Bgra8UnormSrgb, atlas);
        r.ensure_targets(&device, &queue, [320, 200]);
        let map = Arc::new(
            crate::core::defaults::default_project(crate::core::data::Language::default()).maps[0]
                .clone(),
        );
        let input = FrameInput {
            view_proj: Mat4::IDENTITY,
            cam_pos: Vec3::ZERO,
            cam_right: Vec3::X,
            sun_dir: Vec3::new(-0.4, -1.0, -0.3),
            sun_color: [1.0; 3],
            ambient: [0.4; 3],
            fog_color: [0.5; 3],
            fog_density: 0.01,
            darkness: 0.0,
            time: 0.0,
            lights: vec![],
            sprites_cutout: vec![],
            sprites_blend: vec![],
            map,
            map_revision: 1,
            post: PostSettings::default(),
            viewport_px: [320, 200],
        };
        r.ensure_terrain(&device, &queue, &input);
        r.write_uniforms(&queue, &input);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        r.encode(&mut encoder, &input);
        queue.submit([encoder.finish()]);
        device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            })
            .unwrap();
    }
}
