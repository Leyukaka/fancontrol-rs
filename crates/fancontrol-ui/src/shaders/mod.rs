//! Shader-based graph gallery: alternative visual styles for the temperature
//! graph panel, rendered via a custom wgpu pipeline through egui's paint
//! callback mechanism (`egui_wgpu::CallbackTrait`). Only one style renders at
//! a time, so all styles share a single bind group / uniform buffer and only
//! their `RenderPipeline` differs (see `ShaderGallery`).

use crate::graph::ThermalSignal;
use eframe::egui;
use eframe::egui_wgpu::wgpu::util::DeviceExt as _;
use eframe::egui_wgpu::{self, wgpu};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::mem::size_of;
use std::num::NonZeroU64;

const COMMON_WGSL: &str = include_str!("common.wgsl");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphStyle {
    #[default]
    Classic,
    FractalPyramid,
    Plasma,
    LavaBlob,
    Starfield,
}

impl GraphStyle {
    pub const ALL: [GraphStyle; 5] = [
        GraphStyle::Classic,
        GraphStyle::FractalPyramid,
        GraphStyle::Plasma,
        GraphStyle::LavaBlob,
        GraphStyle::Starfield,
    ];

    pub fn is_shader(self) -> bool {
        !matches!(self, GraphStyle::Classic)
    }

    /// The style-specific `fn map(p: vec3<f32>) -> f32` source, concatenated
    /// after `common.wgsl` to build a self-contained shader module. `None`
    /// for `Classic`, which doesn't use a shader at all.
    fn wgsl_fragment(self) -> Option<&'static str> {
        match self {
            GraphStyle::Classic => None,
            GraphStyle::FractalPyramid => Some(include_str!("pyramid.wgsl")),
            GraphStyle::Plasma => Some(include_str!("plasma.wgsl")),
            GraphStyle::LavaBlob => Some(include_str!("lava_blob.wgsl")),
            GraphStyle::Starfield => Some(include_str!("starfield.wgsl")),
        }
    }

    /// i18n key for this style's display name in the Options-panel picker.
    pub fn display_key(self) -> &'static str {
        match self {
            GraphStyle::Classic => "graph_style.classic",
            GraphStyle::FractalPyramid => "graph_style.fractal_pyramid",
            GraphStyle::Plasma => "graph_style.plasma",
            GraphStyle::LavaBlob => "graph_style.lava_blob",
            GraphStyle::Starfield => "graph_style.starfield",
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ShaderUniforms {
    resolution: [f32; 2],
    time: f32,
    cpu01: f32,
    gpu01: f32,
    heat01: f32,
    _pad0: [f32; 2],
    color_a: [f32; 4],
    color_b: [f32; 4],
    /// Reserved for future style-specific tunables; unused today.
    params: [f32; 4],
}

pub struct ShaderGallery {
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    pipelines: HashMap<GraphStyle, wgpu::RenderPipeline>,
}

impl ShaderGallery {
    pub fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Self {
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("shader_gallery"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: NonZeroU64::new(size_of::<ShaderUniforms>() as u64),
                },
                count: None,
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("shader_gallery"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("shader_gallery"),
            contents: bytemuck::cast_slice(&[ShaderUniforms {
                resolution: [1.0, 1.0],
                time: 0.0,
                cpu01: 0.0,
                gpu01: 0.0,
                heat01: 0.0,
                _pad0: [0.0, 0.0],
                color_a: [0.0, 0.0, 0.0, 0.0],
                color_b: [0.0, 0.0, 0.0, 0.0],
                params: [0.0, 0.0, 0.0, 0.0],
            }]),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::UNIFORM,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("shader_gallery"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        let mut pipelines = HashMap::new();
        for style in GraphStyle::ALL {
            let Some(fragment_src) = style.wgsl_fragment() else {
                continue;
            };
            let source = format!("{COMMON_WGSL}\n{fragment_src}");
            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("shader_gallery_style"),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            });
            let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("shader_gallery_style"),
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
            pipelines.insert(style, pipeline);
        }

        Self {
            uniform_buffer,
            bind_group,
            pipelines,
        }
    }

    fn prepare(&self, queue: &wgpu::Queue, uniforms: ShaderUniforms) {
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::cast_slice(&[uniforms]));
    }

    fn paint(&self, style: GraphStyle, render_pass: &mut wgpu::RenderPass<'_>) {
        let Some(pipeline) = self.pipelines.get(&style) else {
            return;
        };
        render_pass.set_pipeline(pipeline);
        render_pass.set_bind_group(0, &self.bind_group, &[]);
        render_pass.draw(0..3, 0..1);
    }
}

struct ShaderCallback {
    style: GraphStyle,
    uniforms: ShaderUniforms,
}

impl egui_wgpu::CallbackTrait for ShaderCallback {
    fn prepare(
        &self,
        _device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen_descriptor: &egui_wgpu::ScreenDescriptor,
        _egui_encoder: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        if let Some(gallery) = resources.get::<ShaderGallery>() {
            gallery.prepare(queue, self.uniforms);
        }
        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        resources: &egui_wgpu::CallbackResources,
    ) {
        if let Some(gallery) = resources.get::<ShaderGallery>() {
            gallery.paint(self.style, render_pass);
        }
    }
}

/// Draw the active shader style's panel. No-op (blank canvas) if the wgpu
/// pipeline wasn't set up at startup, or if `style` somehow has no pipeline.
pub fn show_shader_panel(
    ui: &mut egui::Ui,
    style: GraphStyle,
    time: f32,
    signal: ThermalSignal,
    color_a: [f32; 3],
    color_b: [f32; 3],
) {
    egui::Frame::canvas(ui.style()).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.colored_label(
                crate::graph::temp_color(signal.cpu_c),
                format!("CPU {:.1} °C", signal.cpu_c),
            );
            if signal.gpu_present {
                ui.colored_label(
                    crate::graph::temp_color(signal.gpu_c),
                    format!("GPU {:.1} °C", signal.gpu_c),
                );
            }
        });
        let height = ui.available_height().max(60.0);
        let (rect, _resp) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), height),
            egui::Sense::hover(),
        );
        let resolution = rect.size() * ui.ctx().pixels_per_point();
        let uniforms = ShaderUniforms {
            resolution: [resolution.x, resolution.y],
            time,
            cpu01: signal.cpu01,
            gpu01: signal.gpu01,
            heat01: signal.heat01,
            _pad0: [0.0, 0.0],
            color_a: [color_a[0], color_a[1], color_a[2], 0.0],
            color_b: [color_b[0], color_b[1], color_b[2], 0.0],
            params: [0.0, 0.0, 0.0, 0.0],
        };
        ui.painter().add(egui_wgpu::Callback::new_paint_callback(
            rect,
            ShaderCallback { style, uniforms },
        ));
    });
}
