//! Pictures out of a game: [`png::encode`] writes RGBA8, top row first, as
//! the GPU readback ([`Picture`] or `rrt_gpu::Target::read_back`) gives it.
//! No decoder: game data arrives in the game's own formats, which its crate
//! decodes to RGBA. [`font`] draws text on a picture for overlays.

pub mod font;
pub mod png;

/// An RGBA8 picture, top row first, `width * height * 4` bytes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Picture {
    /// Pixels across.
    pub width: u32,
    /// Rows.
    pub height: u32,
    /// The pixels, R G B A, rows top first with no padding.
    pub rgba: Vec<u8>,
}

impl Picture {
    /// A picture filled with one colour.
    pub fn filled(width: u32, height: u32, rgba: [u8; 4]) -> Picture {
        Picture { width, height, rgba: rgba.repeat((width * height) as usize) }
    }

    /// The pixel at (x, y); None outside.
    pub fn get(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        (x < self.width && y < self.height).then(|| {
            let at = ((y * self.width + x) * 4) as usize;
            self.rgba[at..at + 4].try_into().unwrap()
        })
    }

    /// Sets the pixel at (x, y); nothing outside.
    pub fn set(&mut self, x: u32, y: u32, rgba: [u8; 4]) {
        if x < self.width && y < self.height {
            let at = ((y * self.width + x) * 4) as usize;
            self.rgba[at..at + 4].copy_from_slice(&rgba);
        }
    }

    /// Fills the rectangle at (x, y), `w` x `h`, clipped to the picture.
    pub fn fill_rect(&mut self, x: u32, y: u32, w: u32, h: u32, rgba: [u8; 4]) {
        for row in y..(y + h).min(self.height) {
            for col in x..(x + w).min(self.width) {
                self.set(col, row, rgba);
            }
        }
    }

    /// The picture as a PNG file's bytes.
    pub fn to_png(&self) -> Vec<u8> {
        png::encode(self.width, self.height, &self.rgba)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rects_clip_to_the_picture() {
        let mut p = Picture::filled(4, 4, [0, 0, 0, 255]);
        p.fill_rect(2, 2, 10, 10, [255, 0, 0, 255]);
        assert_eq!(p.get(3, 3), Some([255, 0, 0, 255]));
        assert_eq!(p.get(1, 1), Some([0, 0, 0, 255]));
        assert_eq!(p.get(4, 0), None);
    }
}
