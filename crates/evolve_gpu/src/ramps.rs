//! The ramp atlas (plan D16, task 3.3): the colors of every gradient, one row of 256 texels each.
//!
//! A gradient is drawn by the fragment shader from two things: where the pixel lies along the gradient (a number from 0 to 1,
//! computed from the gradient's geometry, which is in the shape table) and the color at that position, which is looked up
//! here. The colors are made by **asking tiny-skia itself** to draw a 256 x 1 gradient with the same stops, so that the way
//! stops are interpolated and premultiplied is the CPU renderer's own, by construction.
//!
//! A ramp depends only on the stops, so gradients that share their stops share a row, however different their geometry. Rows
//! are made when a draw item first needs them and given back when no item used them in the last frame
//! ([`RampAtlas::begin_frame`] and [`RampAtlas::end_frame`]); a freed row is reused. The atlas is an `Rgba8Unorm` texture of
//! 256 texels by as many rows as needed (it starts at 64 and doubles, up to the device's texture size: 2 048 different
//! gradients on WebGL2, and then a request fails with an error the caller turns into "use the CPU renderer").

use crate::alloc::{Alloc, AllocError, Category, Tracked};
use evolve_engine::style::GradientStop;
use evolve_engine::tiny_skia as sk;
use std::collections::HashMap;

/// Texels in a ramp.
pub const RAMP_TEXELS: u32 = 256;
/// The most stops of one gradient that are used (GPU_SAFETY.md B2): a ramp has 256 texels, so more stops could not be told
/// apart anyway, and a gradient with millions of them must not cost millions of operations.
pub const MAX_STOPS: usize = 256;
const START_ROWS: u32 = 64;
const BYTES_PER_ROW: u64 = RAMP_TEXELS as u64 * 4;

/// What makes two ramps the same: the stops, exactly (position bits and 8-bit color).
type Key = Vec<(u32, [u8; 4])>;

fn key_of(stops: &[GradientStop]) -> Key {
    stops.iter().take(MAX_STOPS).map(|s| (s.offset.to_bits(), [s.color.r, s.color.g, s.color.b, s.color.a])).collect()
}

struct Entry {
    row: u32,
    seen: u64,
}

pub struct RampAtlas {
    alloc: Alloc,
    queue: wgpu::Queue,
    texture: Tracked<wgpu::Texture>,
    view: wgpu::TextureView,
    rows: u32,
    /// What the texture should hold, premultiplied 8-bit RGBA, row after row.
    mirror: Vec<[u8; 4]>,
    dirty: Vec<bool>,
    entries: HashMap<Key, Entry>,
    free: Vec<u32>,
    next_row: u32,
    frame: u64,
    generation: u64,
    uploaded_bytes: u64,
}

/// The 256 colors of a gradient with these stops, as tiny-skia draws it (premultiplied), or `None` when tiny-skia makes no
/// gradient of them (no stops).
pub fn bake(stops: &[GradientStop]) -> Option<Vec<[u8; 4]>> {
    let skia_stops: Vec<sk::GradientStop> =
        stops.iter().take(MAX_STOPS).map(|s| sk::GradientStop::new(s.offset.clamp(0.0, 1.0), sk::Color::from_rgba8(s.color.r, s.color.g, s.color.b, s.color.a))).collect();
    // From x = 0 to x = 256 on a bitmap 256 wide: pixel i is drawn for the position (i + 0.5) / 256.
    let shader = sk::LinearGradient::new(sk::Point::from_xy(0.0, 0.0), sk::Point::from_xy(RAMP_TEXELS as f32, 0.0), skia_stops, sk::SpreadMode::Pad, sk::Transform::identity())?;
    let mut pixmap = sk::Pixmap::new(RAMP_TEXELS, 1)?;
    let paint = sk::Paint { shader, anti_alias: false, ..Default::default() };
    pixmap.fill_rect(sk::Rect::from_xywh(0.0, 0.0, RAMP_TEXELS as f32, 1.0)?, &paint, sk::Transform::identity(), None);
    Some(pixmap.data().as_chunks::<4>().0.to_vec())
}

impl RampAtlas {
    pub fn new(alloc: &Alloc, queue: &wgpu::Queue) -> Result<RampAtlas, AllocError> {
        let (texture, view) = create(alloc, START_ROWS)?;
        Ok(RampAtlas {
            alloc: alloc.clone(),
            queue: queue.clone(),
            texture,
            view,
            rows: START_ROWS,
            mirror: vec![[0; 4]; (START_ROWS * RAMP_TEXELS) as usize],
            dirty: vec![false; START_ROWS as usize],
            entries: HashMap::new(),
            free: Vec::new(),
            next_row: 0,
            frame: 0,
            generation: 1,
            uploaded_bytes: 0,
        })
    }

    pub fn view(&self) -> &wgpu::TextureView {
        &self.view
    }

    /// Changes when the texture is replaced by a larger one (bind groups that use it must be made again).
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Ramps in use.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn capacity_rows(&self) -> u32 {
        self.rows
    }

    pub fn bytes(&self) -> u64 {
        self.texture.bytes()
    }

    pub fn uploaded_bytes(&self) -> u64 {
        self.uploaded_bytes
    }

    /// A frame starts: ramps that nobody asks for until [`RampAtlas::end_frame`] are given back there.
    pub fn begin_frame(&mut self) {
        self.frame += 1;
    }

    /// The row of the ramp for these stops, made now if it is new. `None` when there is no ramp for them (no stops).
    pub fn ensure(&mut self, stops: &[GradientStop]) -> Result<Option<u32>, AllocError> {
        let key = key_of(stops);
        let frame = self.frame;
        if let Some(entry) = self.entries.get_mut(&key) {
            entry.seen = frame;
            return Ok(Some(entry.row));
        }
        let Some(colors) = bake(stops) else { return Ok(None) };
        let row = match self.free.pop() {
            Some(row) => row,
            None => {
                if self.next_row == self.rows {
                    self.grow()?;
                }
                self.next_row += 1;
                self.next_row - 1
            }
        };
        let start = (row * RAMP_TEXELS) as usize;
        self.mirror[start..start + RAMP_TEXELS as usize].copy_from_slice(&colors);
        self.dirty[row as usize] = true;
        self.entries.insert(key, Entry { row, seen: frame });
        Ok(Some(row))
    }

    /// The frame is over: ramps that no item asked for in it are given back.
    pub fn end_frame(&mut self) {
        let frame = self.frame;
        let free = &mut self.free;
        self.entries.retain(|_, entry| {
            let keep = entry.seen == frame;
            if !keep {
                free.push(entry.row);
            }
            keep
        });
    }

    /// Sends the rows that changed to the card, neighbouring rows in one copy. Returns the bytes sent.
    pub fn upload(&mut self) -> u64 {
        let mut sent = 0;
        let mut row = 0;
        while row < self.rows {
            if !self.dirty[row as usize] {
                row += 1;
                continue;
            }
            let first = row;
            while row < self.rows && self.dirty[row as usize] {
                self.dirty[row as usize] = false;
                row += 1;
            }
            let count = row - first;
            let texels = &self.mirror[(first * RAMP_TEXELS) as usize..(row * RAMP_TEXELS) as usize];
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo { texture: &self.texture, mip_level: 0, origin: wgpu::Origin3d { x: 0, y: first, z: 0 }, aspect: wgpu::TextureAspect::All },
                bytemuck::cast_slice(texels),
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(BYTES_PER_ROW as u32), rows_per_image: Some(count) },
                wgpu::Extent3d { width: RAMP_TEXELS, height: count, depth_or_array_layers: 1 },
            );
            sent += count as u64 * BYTES_PER_ROW;
        }
        self.uploaded_bytes += sent;
        sent
    }

    /// Doubles the height of the atlas (up to the limits). The new texture is filled from the CPU copy at the next upload.
    fn grow(&mut self) -> Result<(), AllocError> {
        let limit = self.alloc.device().limits().max_texture_dimension_2d;
        let target = (self.rows * 2).min(limit);
        if target <= self.rows {
            return Err(AllocError::TooLarge { what: "gradient ramps", requested: self.rows as u64 + 1, limit: self.rows as u64 });
        }
        let (texture, view) = create(&self.alloc, target)?;
        self.texture = texture;
        self.view = view;
        self.rows = target;
        self.mirror.resize((target * RAMP_TEXELS) as usize, [0; 4]);
        self.dirty = vec![true; target as usize];
        self.generation += 1;
        Ok(())
    }
}

fn create(alloc: &Alloc, rows: u32) -> Result<(Tracked<wgpu::Texture>, wgpu::TextureView), AllocError> {
    let texture = alloc.create_texture(
        Category::Ramps,
        &wgpu::TextureDescriptor {
            label: Some("gradient ramps"),
            size: wgpu::Extent3d { width: RAMP_TEXELS, height: rows, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        },
    )?;
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    Ok((texture, view))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::Gpu;
    use evolve_engine::style::Color;

    fn two(a: Color, b: Color) -> Vec<GradientStop> {
        vec![GradientStop { offset: 0.0, color: a }, GradientStop { offset: 1.0, color: b }]
    }

    fn stop_at(offset: f32, red: u8) -> GradientStop {
        GradientStop { offset, color: Color::rgb(red, 0, 0) }
    }

    fn on_card(gpu: &Gpu, atlas: &RampAtlas, row: u32) -> Vec<[u8; 4]> {
        let bytes = gpu.read_texture_part(&atlas.texture, 0, row, RAMP_TEXELS, 1);
        bytes.as_chunks::<4>().0.to_vec()
    }

    #[test]
    fn a_ramp_is_what_tiny_skia_draws_for_those_stops_premultiplied() {
        // Black to opaque red: texel i is (i + 0.5) / 256 of the way.
        let ramp = bake(&two(Color::rgb(0, 0, 0), Color::rgb(255, 0, 0))).expect("a ramp");
        assert_eq!(ramp.len(), 256);
        assert!(ramp[0][0] <= 1 && ramp[0][3] == 255, "{:?}", ramp[0]);
        assert!((ramp[128][0] as i32 - 128).abs() <= 1, "{:?}", ramp[128]);
        assert!(ramp[255][0] >= 254);
        // Premultiplied: opaque red to fully transparent red has half the red at the middle, and half the alpha.
        let fade = bake(&two(Color::rgb(255, 0, 0), Color::rgba(255, 0, 0, 0))).expect("a ramp");
        assert!((fade[128][3] as i32 - 127).abs() <= 2 && (fade[128][0] as i32 - 127).abs() <= 2, "{:?}", fade[128]);
        // Several stops: the position of a stop counts.
        let three = bake(&[
            GradientStop { offset: 0.0, color: Color::rgb(255, 0, 0) },
            GradientStop { offset: 0.25, color: Color::rgb(0, 255, 0) },
            GradientStop { offset: 1.0, color: Color::rgb(0, 0, 255) },
        ])
        .expect("a ramp");
        assert!(three[64][1] >= 250 && three[64][0] <= 5 && three[64][2] <= 5, "green at a quarter: {:?}", three[64]);
        // More than 256 stops: the first 256 are used, and the same ramp is shared with a gradient that has just those.
        let many: Vec<GradientStop> = (0..1000).map(|i| stop_at(i as f32 / 999.0, (i % 256) as u8)).collect();
        assert_eq!(bake(&many), bake(&many[..MAX_STOPS]));
        // A single stop is one color everywhere; no stops is no ramp.
        assert!(bake(&[GradientStop { offset: 0.3, color: Color::rgb(9, 8, 7) }]).expect("a ramp").iter().all(|t| *t == [9, 8, 7, 255]));
        assert!(bake(&[]).is_none());
    }

    #[test]
    fn gradients_with_the_same_stops_share_a_row_and_unused_rows_are_given_back_and_reused() {
        Gpu::for_each_test_mode(|gpu| {
            let mut atlas = RampAtlas::new(&gpu.alloc, &gpu.queue).unwrap();
            let (red, blue, green) = (two(Color::rgb(255, 0, 0), Color::BLACK), two(Color::rgb(0, 0, 255), Color::BLACK), two(Color::rgb(0, 255, 0), Color::BLACK));
            atlas.begin_frame();
            let a = atlas.ensure(&red).unwrap().unwrap();
            let b = atlas.ensure(&blue).unwrap().unwrap();
            assert_eq!(atlas.ensure(&red).unwrap(), Some(a), "the same stops, the same row");
            assert_ne!(a, b);
            atlas.end_frame();
            assert_eq!(atlas.len(), 2);
            // Next frame only blue is used: red's row is given back, and a new gradient takes it.
            atlas.begin_frame();
            assert_eq!(atlas.ensure(&blue).unwrap(), Some(b));
            atlas.end_frame();
            assert_eq!(atlas.len(), 1);
            atlas.begin_frame();
            assert_eq!(atlas.ensure(&green).unwrap(), Some(a), "the freed row is reused");
            atlas.end_frame();
            // A ramp that is not asked for in a frame is gone, but one that is asked for again keeps its row meanwhile.
            assert_eq!(atlas.ensure(&[]).unwrap(), None);
        });
    }

    #[test]
    fn the_card_holds_the_ramps_and_only_changed_rows_are_sent() {
        Gpu::for_each_test_mode(|gpu| {
            let mut atlas = RampAtlas::new(&gpu.alloc, &gpu.queue).unwrap();
            atlas.begin_frame();
            let stops = two(Color::rgb(10, 20, 30), Color::rgb(200, 100, 50));
            let row = atlas.ensure(&stops).unwrap().unwrap();
            assert_eq!(atlas.upload(), BYTES_PER_ROW, "one row");
            assert_eq!(atlas.upload(), 0, "nothing changed since");
            assert_eq!(on_card(gpu, &atlas, row), bake(&stops).unwrap());
            let more = two(Color::rgb(1, 2, 3), Color::rgb(250, 251, 252));
            let second = atlas.ensure(&more).unwrap().unwrap();
            assert_eq!(atlas.upload(), BYTES_PER_ROW, "only the new row goes up");
            assert_eq!((on_card(gpu, &atlas, row), on_card(gpu, &atlas, second)), (bake(&stops).unwrap(), bake(&more).unwrap()));
        });
    }

    #[test]
    fn the_atlas_grows_keeps_every_ramp_and_says_so_and_stops_at_the_device_limit_with_an_error() {
        Gpu::for_each_test_mode(|gpu| {
            let mut atlas = RampAtlas::new(&gpu.alloc, &gpu.queue).unwrap();
            let before = atlas.generation();
            atlas.begin_frame();
            let stops = |i: u32| two(Color::rgb((i % 256) as u8, (i / 256) as u8, 7), Color::rgb(255, (i % 200) as u8, 0));
            let rows: Vec<u32> = (0..200).map(|i| atlas.ensure(&stops(i)).unwrap().unwrap()).collect();
            assert!(atlas.capacity_rows() >= 200 && atlas.generation() != before, "the atlas grew and bind groups must be made again");
            atlas.upload();
            for i in [0u32, 63, 64, 199] {
                assert_eq!(on_card(gpu, &atlas, rows[i as usize]), bake(&stops(i)).unwrap(), "ramp {i} after the growth");
            }
            // As many different gradients as the device has rows, and one more: the last one is an error, not a crash.
            let limit = gpu.device.limits().max_texture_dimension_2d;
            let mut failed = None;
            for i in 200..limit + 10 {
                if let Err(error) = atlas.ensure(&stops(i)) {
                    failed = Some((i, error));
                    break;
                }
            }
            let (at, error) = failed.expect("a limit");
            // WebGL2 has 2 048 rows; a native device has more, but the budget of the Ramps category ends it earlier.
            assert!(at >= 2048 && at <= limit, "the atlas holds at least 2 048 gradients and at most the device's rows: {at}");
            assert!(matches!(error, AllocError::TooLarge { .. } | AllocError::OverBudget { .. }), "{error}");
        });
    }
}
