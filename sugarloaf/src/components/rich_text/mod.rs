pub mod doc;
pub mod util;
pub mod layout;
pub mod color;
pub mod text;
mod image_cache;
mod batch;
mod compositor;

use crate::components::core::orthographic_projection;
use crate::context::Context;
use bytemuck::{Pod, Zeroable};
use std::{borrow::Cow, mem};
use wgpu::util::DeviceExt;

const MAX_INSTANCES: usize = 10_000;

#[repr(C)]
#[derive(Debug, Clone, Copy, Zeroable, Pod)]
struct Uniforms {
    transform: [f32; 16],
    scale: f32,
    _padding: [f32; 3],
}

impl Uniforms {
    fn new(transformation: [f32; 16], scale: f32) -> Uniforms {
        Self {
            transform: transformation,
            scale,
            // Ref: https://github.com/iced-rs/iced/blob/bc62013b6cde52174bf4c4286939cf170bfa7760/wgpu/src/quad.rs#LL295C6-L296C68
            // Uniforms must be aligned to their largest member,
            // this uses a mat4x4<f32> which aligns to 16, so align to that
            _padding: [0.0; 3],
        }
    }
}

impl Default for Uniforms {
    fn default() -> Self {
        let identity_matrix: [f32; 16] = [
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0,
            1.0,
        ];
        Self {
            transform: identity_matrix,
            scale: 1.0,
            _padding: [0.0; 3],
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Zeroable, Pod)]
pub struct Vertex {
    _position: [f32; 2],
}

fn vertex(pos: [f32; 2]) -> Vertex {
    Vertex {
        _position: [pos[0], pos[1]],
    }
}

const QUAD_INDICES: [u16; 6] = [0, 1, 2, 0, 2, 3];

#[derive(Debug, Default, Clone, Copy)]
#[repr(C)]
pub struct Rect {
    /// The position of the [`Rect`].
    pub position: [f32; 2],
    pub color: [f32; 4],
    pub size: [f32; 2],
}

#[allow(unsafe_code)]
unsafe impl bytemuck::Zeroable for Rect {}

#[allow(unsafe_code)]
unsafe impl bytemuck::Pod for Rect {}

// TODO: Implement square
fn create_vertices_rect() -> Vec<Vertex> {
    let vertex_data = [
        vertex([0.0, 0.0]),
        vertex([0.5, 0.0]),
        vertex([0.5, 1.0]),
        vertex([0.0, 1.0]),
    ];

    vertex_data.to_vec()
}

pub const BLEND: Option<wgpu::BlendState> = Some(wgpu::BlendState {
    color: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::SrcAlpha,
        dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
        operation: wgpu::BlendOperation::Add,
    },
    alpha: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::One,
        dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
        operation: wgpu::BlendOperation::Add,
    },
});

pub struct RichTextBrush {
    vertex_buf: wgpu::Buffer,
    index_buf: wgpu::Buffer,
    instances: wgpu::Buffer,
    index_count: usize,
    bind_group: wgpu::BindGroup,
    transform: wgpu::Buffer,
    pipeline: wgpu::RenderPipeline,
    current_transform: [f32; 16],
    scale: f32,
}

impl RichTextBrush {
    pub fn new(context: &Context) -> Self {
        let device = &context.device;
        let vertex_data = create_vertices_rect();

        let transform = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: mem::size_of::<Uniforms>() as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let vertex_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Vertex Buffer"),
            contents: bytemuck::cast_slice(&vertex_data),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let index_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Index Buffer"),
            contents: bytemuck::cast_slice(&QUAD_INDICES),
            usage: wgpu::BufferUsages::INDEX,
        });

        // Create pipeline layout
        let bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: None,
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(
                            mem::size_of::<Uniforms>() as wgpu::BufferAddress,
                        ),
                    },
                    count: None,
                }],
            });
        let pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: None,
                bind_group_layouts: &[&bind_group_layout],
                push_constant_ranges: &[],
            });

        // Create bind group
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &transform,
                    offset: 0,
                    size: None,
                }),
            }],
            label: Some("rect::Pipeline uniforms"),
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(include_str!("rich_text.wgsl"))),
        });

        let vertex_buffers = [
            wgpu::VertexBufferLayout {
                array_stride: mem::size_of::<Vertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &[wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x2,
                    offset: 0,
                    shader_location: 0,
                }],
            },
            wgpu::VertexBufferLayout {
                array_stride: mem::size_of::<Rect>() as u64,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &wgpu::vertex_attr_array!(
                    1 => Float32x2,
                    2 => Float32x4,
                    3 => Float32x2,
                ),
            },
        ];

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: None,
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &vertex_buffers,
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "base_fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format: context.format,
                    blend: BLEND,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Cw,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
        });

        let instances = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Instances Buffer"),
            size: mem::size_of::<Rect>() as u64 * MAX_INSTANCES as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Done
        RichTextBrush {
            scale: context.scale,
            vertex_buf,
            index_buf,
            index_count: QUAD_INDICES.len(),
            bind_group,
            transform,
            pipeline,
            current_transform: [0.0; 16],
            instances,
        }
    }
}
