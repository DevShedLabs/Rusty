use rusty_core::Grid;

/// Drives one frame: diff the grid, batch glyph draw calls, submit to GPU.
pub struct FrameRenderer {
    // device:   wgpu::Device,
    // queue:    wgpu::Queue,
    // pipeline: super::pipeline::TerminalPipeline,
    // atlas:    super::atlas::GlyphAtlas,
}

impl FrameRenderer {
    pub fn new() -> Self {
        Self {}
    }

    /// Render `grid` into the current surface frame.
    pub fn render(&mut self, _grid: &Grid) {
        // 1. Walk dirty cells
        // 2. Look up / rasterize glyphs into atlas
        // 3. Build instance buffer (pos, uv, fg, bg)
        // 4. Submit draw call
        // 5. Present
    }
}

impl Default for FrameRenderer {
    fn default() -> Self { Self::new() }
}
