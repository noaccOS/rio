// Copyright (c) 2023-present, Raphael Amorim.
//
// This source code is licensed under the MIT license found in the
// LICENSE file in the root directory of this source tree.
//
// compositor.rs was originally retired from dfrg/swash_demo licensed under MIT
// https://github.com/dfrg/swash_demo/blob/master/LICENSE
//
// Eventually the file had updates to support other features like background-color,
// text color, underline color and etc.

use crate::components::rich_text::batch::BatchManager;
pub use crate::components::rich_text::batch::{
    // Command, DisplayList, Pipeline, Rect, Vertex,
    Command,
    DisplayList,
    Rect,
    Vertex,
};
pub use crate::components::rich_text::image_cache::{
    AddImage,
    ImageId,
    ImageLocation,
    TextureEvent,
    TextureId,
    // AddImage, Epoch, ImageData, ImageId, ImageLocation, TextureEvent, TextureId,
};
use crate::components::rich_text::image_cache::{GlyphCache, ImageCache};
use crate::components::rich_text::text::*;
use crate::SugarCursor;

use std::borrow::Borrow;

pub struct ComposedRect {
    rect: Rect,
    coords: [f32; 4],
    color: [f32; 4],
    has_alpha: bool,
    image: TextureId,
}

pub enum CachedRect {
    Image(ComposedRect),
    Mask(ComposedRect),
    Standard((Rect, [f32; 4])),
}

pub struct Compositor {
    images: ImageCache,
    glyphs: GlyphCache,
    batches: BatchManager,
    intercepts: Vec<(f32, f32)>,
}

impl Compositor {
    /// Creates a new compositor.
    pub fn new(max_texture_size: u16) -> Self {
        Self {
            images: ImageCache::new(max_texture_size),
            glyphs: GlyphCache::new(),
            batches: BatchManager::new(),
            intercepts: Vec::new(),
        }
    }

    /// Advances the epoch for the compositor and clears all batches.
    pub fn begin(&mut self) {
        // TODO: Write a better prune system that doesn't rely on epoch
        // self.glyphs.prune(&mut self.images);
        self.batches.reset();
    }

    /// Builds a display list for the current batched geometry and enumerates
    /// all texture events with the specified closure.
    pub fn finish(&mut self, list: &mut DisplayList, events: impl FnMut(TextureEvent)) {
        self.images.drain_events(events);
        self.batches.build_display_list(list);
    }
}

/// Image management.
impl Compositor {
    /// Adds an image to the compositor.
    #[allow(unused)]
    pub fn add_image(&mut self, request: AddImage) -> Option<ImageId> {
        self.images.allocate(request)
    }

    /// Returns the image associated with the specified identifier.
    #[allow(unused)]
    pub fn get_image(&mut self, image: ImageId) -> Option<ImageLocation> {
        self.images.get(image)
    }

    /// Removes the image from the compositor.
    #[allow(unused)]
    pub fn remove_image(&mut self, image: ImageId) -> bool {
        self.images.deallocate(image).is_some()
    }
}

/// Drawing.
impl Compositor {
    /// Draws a rectangle with the specified depth and color.
    #[allow(unused)]
    pub fn draw_rect(&mut self, rect: impl Into<Rect>, depth: f32, color: &[f32; 4]) {
        self.batches.add_rect(&rect.into(), depth, color);
    }

    /// Draws an image with the specified rectangle, depth and color.
    #[allow(unused)]
    pub fn draw_image(
        &mut self,
        rect: impl Into<Rect>,
        depth: f32,
        color: &[f32; 4],
        image: ImageId,
    ) {
        if let Some(img) = self.images.get(image) {
            self.batches.add_image_rect(
                &rect.into(),
                depth,
                color,
                &[img.min.0, img.min.1, img.max.0, img.max.1],
                img.texture_id,
                image.has_alpha(),
            );
        }
    }

    pub fn draw_glyphs_from_cache(&mut self, cache: &Vec<CachedRect>, depth: f32) {
        for val in cache {
            match val {
                CachedRect::Image(data) => {
                    self.batches.add_image_rect(
                        &data.rect,
                        depth,
                        &data.color,
                        &data.coords,
                        data.image,
                        data.has_alpha,
                    );
                }
                CachedRect::Mask(data) => {
                    self.batches.add_mask_rect(
                        &data.rect,
                        depth,
                        &data.color,
                        &data.coords,
                        data.image,
                        data.has_alpha,
                    );
                }
                CachedRect::Standard((rect, bg_color)) => {
                    self.batches.add_rect(rect, depth, bg_color);
                }
            }
        }
    }

    /// Draws a text run.
    pub fn draw_glyphs<I>(
        &mut self,
        rect: impl Into<Rect>,
        depth: f32,
        style: &TextRunStyle,
        glyphs: I,
        // dimension: SugarDimensions,
    ) -> Vec<CachedRect>
    where
        I: Iterator,
        I::Item: Borrow<Glyph>,
    {
        let rect = rect.into();
        let (underline, underline_offset, underline_size, underline_color) =
            match style.underline {
                Some(underline) => (
                    true,
                    underline.offset.round() as i32,
                    underline.size.round().max(1.),
                    underline.color,
                ),
                _ => (false, 0, 0., [0.0, 0.0, 0.0, 0.0]),
            };
        if underline {
            self.intercepts.clear();
        }
        let mut session = self.glyphs.session(
            &mut self.images,
            style.font,
            style.font_coords,
            style.font_size,
        );
        let mut result = Vec::new();
        let subpx_bias = (0.125, 0.);
        let color = style.color;
        let x = rect.x;
        for g in glyphs {
            let glyph = g.borrow();
            let entry = session.get(glyph.id, glyph.x, glyph.y);
            if let Some(entry) = entry {
                if let Some(img) = session.get_image(entry.image) {
                    let gx = (glyph.x + subpx_bias.0).floor() + entry.left as f32;
                    let gy = (glyph.y + subpx_bias.1).floor() - entry.top as f32;

                    if entry.is_bitmap {
                        let rect =
                            Rect::new(gx, gy, entry.width as f32, entry.height as f32);
                        let color = [1.0, 1.0, 1.0, 1.0];
                        let coords = [img.min.0, img.min.1, img.max.0, img.max.1];
                        self.batches.add_image_rect(
                            &rect,
                            depth,
                            &color,
                            &coords,
                            img.texture_id,
                            entry.image.has_alpha(),
                        );
                        result.push(CachedRect::Image(ComposedRect {
                            rect,
                            color,
                            coords,
                            image: img.texture_id,
                            has_alpha: entry.image.has_alpha(),
                        }));
                    } else {
                        let rect =
                            Rect::new(gx, gy, entry.width as f32, entry.height as f32);
                        let coords = [img.min.0, img.min.1, img.max.0, img.max.1];
                        self.batches.add_mask_rect(
                            &rect,
                            depth,
                            &color,
                            &coords,
                            img.texture_id,
                            true,
                        );
                        result.push(CachedRect::Mask(ComposedRect {
                            rect,
                            color,
                            coords,
                            image: img.texture_id,
                            has_alpha: true,
                        }));
                    }

                    if let Some(bg_color) = style.background_color {
                        let rect = Rect::new(
                            rect.x,
                            style.topline,
                            rect.width,
                            style.line_height,
                        );
                        self.batches.add_rect(&rect, depth, &bg_color);
                        result.push(CachedRect::Standard((rect, bg_color)));
                    }

                    match style.cursor {
                        SugarCursor::Block(cursor_color) => {
                            let rect = Rect::new(
                                rect.x,
                                style.topline,
                                rect.width,
                                style.line_height,
                            );
                            self.batches.add_rect(&rect, depth, &cursor_color);
                            result.push(CachedRect::Standard((rect, cursor_color)));
                        }
                        SugarCursor::Caret(cursor_color) => {
                            let rect =
                                Rect::new(rect.x, style.topline, 3.0, style.line_height);
                            self.batches.add_rect(&rect, depth, &cursor_color);
                            result.push(CachedRect::Standard((rect, cursor_color)));
                        }
                        _ => {}
                    }

                    if underline && entry.top - underline_offset < entry.height as i32 {
                        if let Some(mut desc_ink) = entry.desc.range() {
                            desc_ink.0 += gx;
                            desc_ink.1 += gx;
                            self.intercepts.push(desc_ink);
                        }
                    }
                }
            }
        }
        if underline {
            for range in self.intercepts.iter_mut() {
                range.0 -= 1.;
                range.1 += 1.;
            }
            let mut ux = x;
            let uy = style.baseline - underline_offset as f32;
            for range in self.intercepts.iter() {
                if ux < range.0 {
                    let rect = Rect::new(ux, uy, range.0 - ux, underline_size);
                    self.batches.add_rect(&rect, depth, &underline_color);
                    result.push(CachedRect::Standard((rect, underline_color)));
                }
                ux = range.1;
            }
            let end = x + rect.width;
            if ux < end {
                let rect = Rect::new(ux, uy, end - ux, underline_size);
                self.batches.add_rect(&rect, depth, &underline_color);
                result.push(CachedRect::Standard((rect, underline_color)));
            }
        }

        result
    }
}
