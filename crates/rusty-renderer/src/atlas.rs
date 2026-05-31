use std::collections::HashMap;

/// UV rect within the glyph atlas texture.
#[derive(Debug, Clone, Copy)]
pub struct GlyphRect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// GPU-backed glyph atlas. Rasterize once, cache as a texture region.
pub struct GlyphAtlas {
    cache:       HashMap<(char, u32), GlyphRect>, // (char, font_size_px) → rect
    cursor_x:    u32,
    cursor_y:    u32,
    row_height:  u32,
    pub width:   u32,
    pub height:  u32,
}

impl GlyphAtlas {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            cache:      HashMap::new(),
            cursor_x:   0,
            cursor_y:   0,
            row_height: 0,
            width,
            height,
        }
    }

    pub fn get(&self, ch: char, size_px: u32) -> Option<GlyphRect> {
        self.cache.get(&(ch, size_px)).copied()
    }

    /// Reserve a rect in the atlas. Caller fills the pixel data via wgpu.
    pub fn insert(&mut self, ch: char, size_px: u32, w: u32, h: u32) -> Option<GlyphRect> {
        if self.cursor_x + w > self.width {
            self.cursor_x  = 0;
            self.cursor_y += self.row_height;
            self.row_height = 0;
        }
        if self.cursor_y + h > self.height {
            return None; // atlas full — caller must rebuild
        }
        let rect = GlyphRect { x: self.cursor_x, y: self.cursor_y, w, h };
        self.cache.insert((ch, size_px), rect);
        self.cursor_x += w;
        self.row_height = self.row_height.max(h);
        Some(rect)
    }
}
