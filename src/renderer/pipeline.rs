//! Graphics pipeline for the colored triangle, using dynamic rendering (no
//! `RenderPass`/`Framebuffer`). Shaders are compiled from GLSL at build time by the
//! `vulkano_shaders::shader!` macro and bound by `gl_VertexIndex` — there is no vertex
//! buffer, so the vertex input state is empty.

use std::sync::Arc;

use anyhow::{Context, anyhow};
use vulkano::device::Device;
use vulkano::format::Format;
use vulkano::pipeline::graphics::GraphicsPipelineCreateInfo;
use vulkano::pipeline::graphics::color_blend::{ColorBlendAttachmentState, ColorBlendState};
use vulkano::pipeline::graphics::input_assembly::InputAssemblyState;
use vulkano::pipeline::graphics::multisample::MultisampleState;
use vulkano::pipeline::graphics::rasterization::RasterizationState;
use vulkano::pipeline::graphics::subpass::PipelineRenderingCreateInfo;
use vulkano::pipeline::graphics::vertex_input::VertexInputState;
use vulkano::pipeline::graphics::viewport::ViewportState;
use vulkano::pipeline::layout::PipelineDescriptorSetLayoutCreateInfo;
use vulkano::pipeline::{
    DynamicState, GraphicsPipeline, PipelineLayout, PipelineShaderStageCreateInfo,
};

mod vs {
    vulkano_shaders::shader! { ty: "vertex", path: "src/shaders/triangle.vert" }
}

mod fs {
    vulkano_shaders::shader! { ty: "fragment", path: "src/shaders/triangle.frag" }
}

/// Build the triangle pipeline for the given swapchain image format. Rebuilt only if
/// the format changes (resize keeps the format, so the pipeline is reused).
pub fn create_pipeline(
    device: Arc<Device>,
    swapchain_format: Format,
) -> anyhow::Result<Arc<GraphicsPipeline>> {
    let vs = vs::load(device.clone())
        .context("loading vertex shader")?
        .entry_point("main")
        .ok_or_else(|| anyhow!("vertex shader has no `main` entry point"))?;
    let fs = fs::load(device.clone())
        .context("loading fragment shader")?
        .entry_point("main")
        .ok_or_else(|| anyhow!("fragment shader has no `main` entry point"))?;

    let stages = [
        PipelineShaderStageCreateInfo::new(vs),
        PipelineShaderStageCreateInfo::new(fs),
    ];

    let layout = PipelineLayout::new(
        device.clone(),
        PipelineDescriptorSetLayoutCreateInfo::from_stages(&stages)
            .into_pipeline_layout_create_info(device.clone())
            .context("building pipeline layout create info")?,
    )
    .context("creating pipeline layout")?;

    // Dynamic-rendering hookup: this replaces a render pass/subpass. The pipeline must
    // know the color attachment format up front.
    let subpass = PipelineRenderingCreateInfo {
        color_attachment_formats: vec![Some(swapchain_format)],
        ..Default::default()
    };

    GraphicsPipeline::new(
        device.clone(),
        None,
        GraphicsPipelineCreateInfo {
            stages: stages.into_iter().collect(),
            // Positions + colors are baked into the vertex shader; no vertex buffer.
            vertex_input_state: Some(VertexInputState::default()),
            input_assembly_state: Some(InputAssemblyState::default()),
            viewport_state: Some(ViewportState::default()),
            rasterization_state: Some(RasterizationState::default()),
            multisample_state: Some(MultisampleState::default()),
            color_blend_state: Some(ColorBlendState::with_attachment_states(
                subpass.color_attachment_formats.len() as u32,
                ColorBlendAttachmentState::default(),
            )),
            // Viewport is set per-frame so a resize never rebuilds the pipeline.
            dynamic_state: [DynamicState::Viewport].into_iter().collect(),
            subpass: Some(subpass.into()),
            ..GraphicsPipelineCreateInfo::layout(layout)
        },
    )
    .context("creating graphics pipeline")
}
