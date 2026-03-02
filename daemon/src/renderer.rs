// 2D renderer for notification cards.
// Provides primitives: solid rectangles, textured quads, and text drawing.

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use crate::gpu::GpuContext;

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct Vertex {
    pub position: [f32; 2],
    pub tex_coords: [f32; 2],
    pub color: [f32; 4],
}

impl Vertex {
    const ATTRIBS: [wgpu::VertexAttribute; 3] = wgpu::vertex_attr_array![
        0 => Float32x2,
        1 => Float32x2,
        2 => Float32x4,
    ];

    pub fn desc() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBS,
        }
    }
}

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct Uniforms {
    projection: [[f32; 4]; 4],
}

/// Batched 2D renderer for quads and text.
pub struct Renderer2D {
    solid_pipeline: wgpu::RenderPipeline,
    textured_pipeline: wgpu::RenderPipeline,
    uniform_buffer: wgpu::Buffer,
    uniform_bind_group: wgpu::BindGroup,
    texture_bind_group_layout: wgpu::BindGroupLayout,

    // White 1x1 texture for solid color rendering via textured pipeline
    white_texture_bind_group: wgpu::BindGroup,

    // Per-frame batch
    vertices: Vec<Vertex>,
    indices: Vec<u32>,
    draw_calls: Vec<DrawCall>,
}

enum DrawCall {
    Solid {
        index_offset: u32,
        index_count: u32,
    },
    Textured {
        index_offset: u32,
        index_count: u32,
        bind_group: Arc<wgpu::BindGroup>,
    },
    /// Text batch: uses the font atlas bind group (bound externally).
    TextBatch {
        index_offset: u32,
        index_count: u32,
    },
}

impl Renderer2D {
    pub fn new(gpu: &GpuContext) -> Self {
        let shader = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("Quad Shader"),
                source: wgpu::ShaderSource::Wgsl(
                    include_str!("../shaders/quad.wgsl").into(),
                ),
            });

        // Uniform bind group layout (projection matrix)
        let uniform_bind_group_layout =
            gpu.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("Uniform Bind Group Layout"),
                    entries: &[wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    }],
                });

        // Texture bind group layout
        let texture_bind_group_layout =
            gpu.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("Texture Bind Group Layout"),
                    entries: &[
                        wgpu::BindGroupLayoutEntry {
                            binding: 0,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Texture {
                                multisampled: false,
                                view_dimension: wgpu::TextureViewDimension::D2,
                                sample_type: wgpu::TextureSampleType::Float {
                                    filterable: true,
                                },
                            },
                            count: None,
                        },
                        wgpu::BindGroupLayoutEntry {
                            binding: 1,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Sampler(
                                wgpu::SamplerBindingType::Filtering,
                            ),
                            count: None,
                        },
                    ],
                });

        let pipeline_layout =
            gpu.device
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("Quad Pipeline Layout"),
                    bind_group_layouts: &[
                        &uniform_bind_group_layout,
                        &texture_bind_group_layout,
                    ],
                    push_constant_ranges: &[],
                });

        let targets = &[Some(wgpu::ColorTargetState {
            format: gpu.surface_format(),
            blend: Some(wgpu::BlendState::ALPHA_BLENDING),
            write_mask: wgpu::ColorWrites::ALL,
        })];

        let solid_pipeline =
            gpu.device
                .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("Solid Pipeline"),
                    layout: Some(&pipeline_layout),
                    vertex: wgpu::VertexState {
                        module: &shader,
                        entry_point: Some("vs_main"),
                        buffers: &[Vertex::desc()],
                        compilation_options: Default::default(),
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &shader,
                        entry_point: Some("fs_solid"),
                        targets,
                        compilation_options: Default::default(),
                    }),
                    primitive: wgpu::PrimitiveState {
                        topology: wgpu::PrimitiveTopology::TriangleList,
                        ..Default::default()
                    },
                    depth_stencil: None,
                    multisample: wgpu::MultisampleState::default(),
                    multiview: None,
                    cache: None,
                });

        let textured_pipeline =
            gpu.device
                .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("Textured Pipeline"),
                    layout: Some(&pipeline_layout),
                    vertex: wgpu::VertexState {
                        module: &shader,
                        entry_point: Some("vs_main"),
                        buffers: &[Vertex::desc()],
                        compilation_options: Default::default(),
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &shader,
                        entry_point: Some("fs_main"),
                        targets,
                        compilation_options: Default::default(),
                    }),
                    primitive: wgpu::PrimitiveState {
                        topology: wgpu::PrimitiveTopology::TriangleList,
                        ..Default::default()
                    },
                    depth_stencil: None,
                    multisample: wgpu::MultisampleState::default(),
                    multiview: None,
                    cache: None,
                });

        // Create uniform buffer with placeholder projection (updated on resize)
        let projection = orthographic_projection(1.0, 1.0);
        let uniform_buffer =
            gpu.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("Uniform Buffer"),
                    contents: bytemuck::cast_slice(&[Uniforms { projection }]),
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                });

        let uniform_bind_group =
            gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Uniform Bind Group"),
                layout: &uniform_bind_group_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                }],
            });

        // Create 1x1 white texture for solid color rendering
        let white_texture_bind_group =
            create_white_texture(&gpu.device, &gpu.queue, &texture_bind_group_layout);

        Self {
            solid_pipeline,
            textured_pipeline,
            uniform_buffer,
            uniform_bind_group,
            texture_bind_group_layout,
            white_texture_bind_group,
            vertices: Vec::with_capacity(4096),
            indices: Vec::with_capacity(6144),
            draw_calls: Vec::with_capacity(64),
        }
    }

    pub fn resize(&self, gpu: &GpuContext, width: u32, height: u32) {
        let projection = orthographic_projection(width as f32, height as f32);
        gpu.queue.write_buffer(
            &self.uniform_buffer,
            0,
            bytemuck::cast_slice(&[Uniforms { projection }]),
        );
    }

    /// Begin a new frame. Clears all batched geometry.
    pub fn begin_frame(&mut self) {
        self.vertices.clear();
        self.indices.clear();
        self.draw_calls.clear();
    }

    /// Draw a solid color rectangle.
    pub fn draw_rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: [f32; 4]) {
        let vertex_offset = self.vertices.len() as u32;
        let index_offset = self.indices.len() as u32;

        self.vertices.extend_from_slice(&[
            Vertex { position: [x, y], tex_coords: [0.0, 0.0], color },
            Vertex { position: [x + w, y], tex_coords: [1.0, 0.0], color },
            Vertex { position: [x + w, y + h], tex_coords: [1.0, 1.0], color },
            Vertex { position: [x, y + h], tex_coords: [0.0, 1.0], color },
        ]);

        self.indices.extend_from_slice(&[
            vertex_offset,
            vertex_offset + 1,
            vertex_offset + 2,
            vertex_offset,
            vertex_offset + 2,
            vertex_offset + 3,
        ]);

        self.draw_calls.push(DrawCall::Solid {
            index_offset,
            index_count: 6,
        });
    }

    /// Draw a textured rectangle. The bind_group should contain the texture and sampler.
    pub fn draw_textured(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        color: [f32; 4],
        bind_group: &Arc<wgpu::BindGroup>,
    ) {
        let vertex_offset = self.vertices.len() as u32;
        let index_offset = self.indices.len() as u32;

        self.vertices.extend_from_slice(&[
            Vertex { position: [x, y], tex_coords: [0.0, 0.0], color },
            Vertex { position: [x + w, y], tex_coords: [1.0, 0.0], color },
            Vertex { position: [x + w, y + h], tex_coords: [1.0, 1.0], color },
            Vertex { position: [x, y + h], tex_coords: [0.0, 1.0], color },
        ]);

        self.indices.extend_from_slice(&[
            vertex_offset,
            vertex_offset + 1,
            vertex_offset + 2,
            vertex_offset,
            vertex_offset + 2,
            vertex_offset + 3,
        ]);

        self.draw_calls.push(DrawCall::Textured {
            index_offset,
            index_count: 6,
            bind_group: bind_group.clone(),
        });
    }

    /// Upload an RGBA image to a GPU texture and return a bind group for it.
    pub fn upload_texture(
        &self,
        gpu: &GpuContext,
        image: &image::RgbaImage,
    ) -> Arc<wgpu::BindGroup> {
        let (width, height) = image.dimensions();
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Notification Icon"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        gpu.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            image.as_raw(),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * width),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        Arc::new(gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Texture Bind Group"),
            layout: &self.texture_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        }))
    }

    /// Create a bind group for an existing texture (used by font atlas).
    pub fn create_texture_bind_group(
        &self,
        gpu: &GpuContext,
        texture: &wgpu::Texture,
    ) -> wgpu::BindGroup {
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Font Atlas Bind Group"),
            layout: &self.texture_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        })
    }

    /// Batch text vertices (pre-generated by FontAtlas) as a single textured draw call.
    /// The bind_group should be the font atlas texture.
    pub fn draw_text_batch(
        &mut self,
        text_vertices: &[Vertex],
        text_indices: &[u32],
        _bind_group: &wgpu::BindGroup,
    ) {
        if text_vertices.is_empty() {
            return;
        }

        let vertex_offset = self.vertices.len() as u32;
        let index_offset = self.indices.len() as u32;

        self.vertices.extend_from_slice(text_vertices);

        // Remap indices relative to our vertex buffer
        for &idx in text_indices {
            self.indices.push(idx + vertex_offset);
        }

        // We can't clone bind groups, so we use a reference trick:
        // For now, we'll just note we need the textured pipeline with the atlas.
        // We'll handle this by accepting the bind group index pattern.
        // Actually, we need owned bind groups in draw calls. Let's use a sentinel approach:
        // Store the index range and we'll bind the atlas externally.
        self.draw_calls.push(DrawCall::TextBatch {
            index_offset,
            index_count: text_indices.len() as u32,
        });
    }

    /// Submit all batched draw calls to the GPU.
    /// `font_atlas_bind_group` is used for TextBatch draw calls.
    pub fn render(
        &self,
        gpu: &GpuContext,
        view: &wgpu::TextureView,
        font_atlas_bind_group: Option<&wgpu::BindGroup>,
    ) {
        if self.vertices.is_empty() {
            return;
        }

        let vertex_buffer =
            gpu.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("Vertex Buffer"),
                    contents: bytemuck::cast_slice(&self.vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                });

        let index_buffer =
            gpu.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("Index Buffer"),
                    contents: bytemuck::cast_slice(&self.indices),
                    usage: wgpu::BufferUsages::INDEX,
                });

        let mut encoder =
            gpu.device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("Render Encoder"),
                });

        {
            let mut render_pass =
                encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Main Render Pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    ..Default::default()
                });

            render_pass.set_vertex_buffer(0, vertex_buffer.slice(..));
            render_pass.set_index_buffer(
                index_buffer.slice(..),
                wgpu::IndexFormat::Uint32,
            );
            render_pass.set_bind_group(0, &self.uniform_bind_group, &[]);

            for call in &self.draw_calls {
                match call {
                    DrawCall::Solid {
                        index_offset,
                        index_count,
                        ..
                    } => {
                        render_pass.set_pipeline(&self.solid_pipeline);
                        // Still need to bind texture group for layout compatibility
                        render_pass.set_bind_group(
                            1,
                            &self.white_texture_bind_group,
                            &[],
                        );
                        let range =
                            *index_offset..(*index_offset + *index_count);
                        render_pass.draw_indexed(range, 0, 0..1);
                    }
                    DrawCall::Textured {
                        index_offset,
                        index_count,
                        bind_group,
                    } => {
                        render_pass.set_pipeline(&self.textured_pipeline);
                        render_pass.set_bind_group(1, &**bind_group, &[]);
                        let range =
                            *index_offset..(*index_offset + *index_count);
                        render_pass.draw_indexed(range, 0, 0..1);
                    }
                    DrawCall::TextBatch {
                        index_offset,
                        index_count,
                    } => {
                        if let Some(atlas_bg) = font_atlas_bind_group {
                            render_pass.set_pipeline(&self.textured_pipeline);
                            render_pass.set_bind_group(1, atlas_bg, &[]);
                            let range =
                                *index_offset..(*index_offset + *index_count);
                            render_pass.draw_indexed(range, 0, 0..1);
                        }
                    }
                }
            }
        }

        gpu.queue.submit(std::iter::once(encoder.finish()));
    }
}

/// Creates a 2D orthographic projection matrix (top-left origin).
fn orthographic_projection(width: f32, height: f32) -> [[f32; 4]; 4] {
    [
        [2.0 / width, 0.0, 0.0, 0.0],
        [0.0, -2.0 / height, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [-1.0, 1.0, 0.0, 1.0],
    ]
}

/// Creates a 1x1 white texture for solid color rendering.
fn create_white_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
) -> wgpu::BindGroup {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("White Texture"),
        size: wgpu::Extent3d {
            width: 1,
            height: 1,
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
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &[255, 255, 255, 255],
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(4),
            rows_per_image: Some(1),
        },
        wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
    );

    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor::default());

    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("White Texture Bind Group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
        ],
    })
}
