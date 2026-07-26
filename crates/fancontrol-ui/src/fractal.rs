//! Optional "fun" panel: a raymarched fractal, rendered via a custom wgpu
//! pipeline through egui's paint-callback mechanism (see `app.rs` for the
//! one-time pipeline setup and per-frame wiring).

use eframe::egui;
use eframe::egui_wgpu::wgpu::util::DeviceExt as _;
use eframe::egui_wgpu::{self, wgpu};
use std::mem::size_of;
use std::num::NonZeroU64;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct FractalUniforms {
    resolution: [f32; 2],
    time: f32,
    _pad0: f32,
    color_a: [f32; 4],
    color_b: [f32; 4],
}

pub struct FractalResources {
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    uniform_buffer: wgpu::Buffer,
}

impl FractalResources {
    pub fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("fractal"),
            source: wgpu::ShaderSource::Wgsl(include_str!("./fractal_shader.wgsl").into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fractal"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: NonZeroU64::new(size_of::<FractalUniforms>() as u64),
                },
                count: None,
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("fractal"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("fractal"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(target_format.into())],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("fractal"),
            contents: bytemuck::cast_slice(&[FractalUniforms {
                resolution: [1.0, 1.0],
                time: 0.0,
                _pad0: 0.0,
                color_a: [0.0, 0.0, 0.0, 0.0],
                color_b: [0.0, 0.0, 0.0, 0.0],
            }]),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::UNIFORM,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fractal"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        Self {
            pipeline,
            bind_group,
            uniform_buffer,
        }
    }

    fn prepare(
        &self,
        queue: &wgpu::Queue,
        time: f32,
        resolution: [f32; 2],
        color_a: [f32; 3],
        color_b: [f32; 3],
    ) {
        let uniforms = FractalUniforms {
            resolution,
            time,
            _pad0: 0.0,
            color_a: [color_a[0], color_a[1], color_a[2], 0.0],
            color_b: [color_b[0], color_b[1], color_b[2], 0.0],
        };
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::cast_slice(&[uniforms]));
    }

    fn paint(&self, render_pass: &mut wgpu::RenderPass<'_>) {
        render_pass.set_pipeline(&self.pipeline);
        render_pass.set_bind_group(0, &self.bind_group, &[]);
        render_pass.draw(0..3, 0..1);
    }
}

struct FractalCallback {
    time: f32,
    resolution: [f32; 2],
    color_a: [f32; 3],
    color_b: [f32; 3],
}

impl egui_wgpu::CallbackTrait for FractalCallback {
    fn prepare(
        &self,
        _device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen_descriptor: &egui_wgpu::ScreenDescriptor,
        _egui_encoder: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        if let Some(res) = resources.get::<FractalResources>() {
            res.prepare(
                queue,
                self.time,
                self.resolution,
                self.color_a,
                self.color_b,
            );
        }
        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        resources: &egui_wgpu::CallbackResources,
    ) {
        if let Some(res) = resources.get::<FractalResources>() {
            res.paint(render_pass);
        }
    }
}

/// Draw the fractal panel. No-op (blank canvas) if the wgpu pipeline wasn't set up at startup.
pub fn show_fractal_panel(ui: &mut egui::Ui, time: f32, color_a: [f32; 3], color_b: [f32; 3]) {
    egui::Frame::canvas(ui.style()).show(ui, |ui| {
        let (rect, _resp) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), 220.0),
            egui::Sense::hover(),
        );
        let resolution = rect.size() * ui.ctx().pixels_per_point();
        ui.painter().add(egui_wgpu::Callback::new_paint_callback(
            rect,
            FractalCallback {
                time,
                resolution: [resolution.x, resolution.y],
                color_a,
                color_b,
            },
        ));
    });
}
