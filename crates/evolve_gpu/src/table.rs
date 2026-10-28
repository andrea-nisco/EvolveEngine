//! The shape table (plan D10, task 2.3): a float texture used as a table, one row per draw item.
//!
//! A uniform buffer cannot hold per-shape data on WebGL2 (16 KB is room for about 250 transforms), so the table is an
//! `Rgba32Float` texture that the shader reads with `textureLoad`. A row is eight texels of four floats:
//!
//! | texel | contents |
//! |---|---|
//! | 0 | the 2x2 matrix of the item's world transform: `x_axis.x, x_axis.y, y_axis.x, y_axis.y` (for a layer's composite: its rectangle `x, y, width, height` in pixels) |
//! | 1 | the translation `x, y`; the depth value `z`; the paint kind `w` (0 = solid color, 1 = linear gradient, 2 = radial, 3 = a layer's composite) |
//! | 2 | the premultiplied color `r, g, b, a` (white for a gradient, so that the opacity multiplies it) |
//! | 3 | the stroke width, flags, the opacity, one spare |
//! | 4 | the gradient: linear `a, b, c` so that the position along it is `a x + b y + c` for a point `(x, y)` of the shape's own space; radial the 2x2 matrix `m00, m01, m10, m11` |
//! | 5 | the gradient: radial the offset `x, y` (the position is the length of `M p + offset`); the ramp's row `z` in the ramp texture |
//! | 6, 7 | spare |
//!
//! The table is 1 024 texels wide, so a line of it holds 128 rows. Moving, recoloring or fading a shape rewrites one row.
//! The CPU keeps a copy of the table; a row that is set to what it already holds costs nothing, and the lines that did
//! change are uploaded together in [`ShapeTable::upload`]. The table starts small and doubles its height when rows run
//! out, up to the device's texture size limit (2 048 lines on WebGL2: over half a million rows).

use crate::alloc::{Alloc, AllocError, Category, Tracked};
use evolve_engine::geometry::Affine2;
use evolve_engine::style::Color;

pub const ROW_TEXELS: u32 = 8;
pub const LINE_TEXELS: u32 = 1024;
pub const ROWS_PER_LINE: u32 = LINE_TEXELS / ROW_TEXELS;
const BYTES_PER_LINE: u64 = LINE_TEXELS as u64 * 16;
const START_LINES: u32 = 16;

/// What a draw item is painted with.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PaintKind {
    Solid,
    Linear,
    Radial,
    /// Not a paint: the composite of a layer (see [`RowData::layer`]).
    Layer,
}

impl PaintKind {
    fn code(self) -> f32 {
        match self {
            PaintKind::Solid => 0.0,
            PaintKind::Linear => 1.0,
            PaintKind::Radial => 2.0,
            PaintKind::Layer => 3.0,
        }
    }
}

/// One row of the table: 32 floats.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct RowData(pub [f32; 32]);

impl RowData {
    /// The row of a draw item with a solid color. `color` is premultiplied (see [`premultiplied`]).
    pub fn solid(world: &Affine2, depth: f32, color: [f32; 4], stroke_width: f32, opacity: f32) -> RowData {
        let m = world.matrix2;
        let mut row = [0.0; 32];
        row[..16].copy_from_slice(&[
            m.x_axis.x, m.x_axis.y, m.y_axis.x, m.y_axis.y, //
            world.translation.x, world.translation.y, depth, PaintKind::Solid.code(), //
            color[0], color[1], color[2], color[3], //
            stroke_width, 0.0, opacity, 0.0,
        ]);
        RowData(row)
    }

    /// The row of a draw item painted with a gradient: `geometry` is the two gradient texels (see the table above) and `ramp`
    /// the row of its colors in the ramp texture.
    pub fn gradient(world: &Affine2, depth: f32, kind: PaintKind, geometry: [f32; 8], ramp: u32, stroke_width: f32, opacity: f32) -> RowData {
        let mut row = RowData::solid(world, depth, [1.0; 4], stroke_width, opacity);
        row.0[7] = kind.code();
        row.0[16..24].copy_from_slice(&geometry);
        row.0[22] = ramp as f32;
        row
    }

    /// The row of a layer's composite: a rectangle of `rect` = `x, y, width, height` pixels in the pass it is drawn in, showing
    /// the layer's texture at `opacity`. Texel 0 holds the rectangle (in place of a matrix), the color is white so that the
    /// opacity scales the layer's premultiplied texels. `ratio` is the layer's texels for each pixel of the pass (1 for a layer drawn
    /// at its own size).
    pub fn layer(rect: [f32; 4], depth: f32, opacity: f32, ratio: f32) -> RowData {
        let mut row = [0.0; 32];
        row[..16].copy_from_slice(&[
            rect[0], rect[1], rect[2], rect[3], //
            0.0, 0.0, depth, PaintKind::Layer.code(), //
            1.0, 1.0, 1.0, 1.0, //
            0.0, 0.0, opacity, ratio,
        ]);
        RowData(row)
    }

    pub fn kind(&self) -> f32 {
        self.0[7]
    }

    pub fn ramp(&self) -> u32 {
        self.0[22] as u32
    }

    pub fn depth(&self) -> f32 {
        self.0[6]
    }

    pub fn color(&self) -> [f32; 4] {
        [self.0[8], self.0[9], self.0[10], self.0[11]]
    }
}

/// A color as the card blends it: red, green and blue multiplied by alpha, all between 0 and 1.
pub fn premultiplied(color: Color) -> [f32; 4] {
    let a = color.a as f32 / 255.0;
    [color.r as f32 / 255.0 * a, color.g as f32 / 255.0 * a, color.b as f32 / 255.0 * a, a]
}

pub struct ShapeTable {
    alloc: Alloc,
    queue: wgpu::Queue,
    texture: Tracked<wgpu::Texture>,
    view: wgpu::TextureView,
    lines: u32,
    mirror: Vec<RowData>,
    dirty_lines: Vec<bool>,
    free: Vec<u32>,
    next_row: u32,
    generation: u64,
    uploaded_bytes: u64,
}

impl ShapeTable {
    pub fn new(alloc: &Alloc, queue: &wgpu::Queue) -> Result<ShapeTable, AllocError> {
        let (texture, view) = create(alloc, START_LINES)?;
        Ok(ShapeTable {
            alloc: alloc.clone(),
            queue: queue.clone(),
            texture,
            view,
            lines: START_LINES,
            mirror: vec![RowData([0.0; 32]); (START_LINES * ROWS_PER_LINE) as usize],
            dirty_lines: vec![false; START_LINES as usize],
            free: Vec::new(),
            next_row: 0,
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

    pub fn capacity_rows(&self) -> u32 {
        self.lines * ROWS_PER_LINE
    }

    pub fn rows_in_use(&self) -> u32 {
        self.next_row - self.free.len() as u32
    }

    /// Bytes uploaded to the table so far.
    pub fn uploaded_bytes(&self) -> u64 {
        self.uploaded_bytes
    }

    pub fn bytes(&self) -> u64 {
        self.texture.bytes()
    }

    pub fn get(&self, row: u32) -> &RowData {
        &self.mirror[row as usize]
    }

    /// Hands out a free row, making the table taller when it is full.
    pub fn alloc_row(&mut self) -> Result<u32, AllocError> {
        if let Some(row) = self.free.pop() {
            return Ok(row);
        }
        if self.next_row == self.capacity_rows() {
            self.grow()?;
        }
        let row = self.next_row;
        self.next_row += 1;
        Ok(row)
    }

    pub fn free_row(&mut self, row: u32) {
        debug_assert!(row < self.next_row && !self.free.contains(&row), "a row that is not in use");
        self.free.push(row);
    }

    /// Writes a row. Returns false, and does nothing, when the row already holds exactly this.
    pub fn set(&mut self, row: u32, data: RowData) -> bool {
        let slot = &mut self.mirror[row as usize];
        if *slot == data {
            return false;
        }
        *slot = data;
        self.dirty_lines[(row / ROWS_PER_LINE) as usize] = true;
        true
    }

    /// Sends the lines that changed to the card, neighbouring lines in one copy. Returns the bytes sent.
    pub fn upload(&mut self) -> u64 {
        let mut sent = 0;
        let mut line = 0;
        while line < self.lines {
            if !self.dirty_lines[line as usize] {
                line += 1;
                continue;
            }
            let first = line;
            while line < self.lines && self.dirty_lines[line as usize] {
                self.dirty_lines[line as usize] = false;
                line += 1;
            }
            let count = line - first;
            let rows = &self.mirror[(first * ROWS_PER_LINE) as usize..(line * ROWS_PER_LINE) as usize];
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo { texture: &self.texture, mip_level: 0, origin: wgpu::Origin3d { x: 0, y: first, z: 0 }, aspect: wgpu::TextureAspect::All },
                bytemuck::cast_slice(rows),
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(BYTES_PER_LINE as u32), rows_per_image: Some(count) },
                wgpu::Extent3d { width: LINE_TEXELS, height: count, depth_or_array_layers: 1 },
            );
            sent += count as u64 * BYTES_PER_LINE;
        }
        self.uploaded_bytes += sent;
        sent
    }

    /// Doubles the height of the table (up to the limits). The new texture is filled from the CPU copy at the next upload.
    fn grow(&mut self) -> Result<(), AllocError> {
        let limit = self.alloc.device().limits().max_texture_dimension_2d;
        let target = (self.lines * 2).min(limit);
        if target <= self.lines {
            return Err(AllocError::TooLarge { what: "shape table rows", requested: self.capacity_rows() as u64 + 1, limit: self.capacity_rows() as u64 });
        }
        let (texture, view) = create(&self.alloc, target)?;
        self.texture = texture;
        self.view = view;
        self.lines = target;
        self.mirror.resize((target * ROWS_PER_LINE) as usize, RowData([0.0; 32]));
        self.dirty_lines = vec![true; target as usize];
        self.generation += 1;
        Ok(())
    }
}

fn create(alloc: &Alloc, lines: u32) -> Result<(Tracked<wgpu::Texture>, wgpu::TextureView), AllocError> {
    let texture = alloc.create_texture(
        Category::ShapeTable,
        &wgpu::TextureDescriptor {
            label: Some("shape table"),
            size: wgpu::Extent3d { width: LINE_TEXELS, height: lines, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba32Float,
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
    use evolve_engine::Vec2;

    fn row(seed: f32) -> RowData {
        RowData::solid(&Affine2::from_translation(Vec2::new(seed, -seed)), seed * 0.001, [seed, 0.5, 0.25, 1.0], seed + 1.0, 0.75)
    }

    /// What the card holds for a row, read back from the texture.
    fn on_card(gpu: &Gpu, table: &ShapeTable, row: u32) -> [f32; 32] {
        let bytes = gpu.read_texture_part(&table.texture, (row % ROWS_PER_LINE) * ROW_TEXELS, row / ROWS_PER_LINE, ROW_TEXELS, 1);
        bytemuck::cast_slice::<u8, f32>(&bytes).try_into().expect("32 floats")
    }

    #[test]
    fn a_row_is_eight_texels_with_the_matrix_translation_depth_color_width_and_gradient_in_place() {
        let world = Affine2::from_cols_array(&[2.0, 3.0, 5.0, 7.0, 11.0, 13.0]);
        let data = RowData::solid(&world, 0.125, [0.1, 0.2, 0.3, 0.4], 6.0, 0.5);
        let mut expected = [0.0; 32];
        expected[..16].copy_from_slice(&[2.0, 3.0, 5.0, 7.0, 11.0, 13.0, 0.125, 0.0, 0.1, 0.2, 0.3, 0.4, 6.0, 0.0, 0.5, 0.0]);
        assert_eq!(data.0, expected);
        // A gradient row: the same first four texels with white for the color, the paint kind, the two gradient texels, the ramp.
        let gradient = RowData::gradient(&world, 0.125, PaintKind::Radial, [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 0.0, 0.0], 77, 6.0, 0.5);
        assert_eq!(&gradient.0[..16], &[2.0, 3.0, 5.0, 7.0, 11.0, 13.0, 0.125, 2.0, 1.0, 1.0, 1.0, 1.0, 6.0, 0.0, 0.5, 0.0]);
        assert_eq!(&gradient.0[16..24], &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 77.0, 0.0]);
        assert_eq!((gradient.kind(), gradient.ramp()), (2.0, 77));
        assert_eq!((data.depth(), data.color()), (0.125, [0.1, 0.2, 0.3, 0.4]));
        assert_eq!(premultiplied(Color::rgba(255, 128, 0, 128)), [128.0 / 255.0, 128.0 / 255.0 * 128.0 / 255.0, 0.0, 128.0 / 255.0]);
        assert_eq!(premultiplied(Color::rgb(10, 20, 30)), [10.0 / 255.0, 20.0 / 255.0, 30.0 / 255.0, 1.0]);
        assert_eq!(std::mem::size_of::<RowData>(), 128);
    }

    #[test]
    fn rows_are_handed_out_reused_and_written_only_when_they_change() {
        Gpu::for_each_test_mode(|gpu| {
            let mut table = ShapeTable::new(&gpu.alloc, &gpu.queue).unwrap();
            let (a, b, c) = (table.alloc_row().unwrap(), table.alloc_row().unwrap(), table.alloc_row().unwrap());
            assert_eq!((a, b, c), (0, 1, 2));
            assert_eq!(table.rows_in_use(), 3);
            table.free_row(b);
            assert_eq!(table.alloc_row().unwrap(), b, "a freed row is reused");
            assert!(table.set(a, row(1.0)), "a new value is a change");
            assert!(!table.set(a, row(1.0)), "the same value again is not");
            assert!(table.set(a, row(2.0)));
            assert_eq!(table.get(a), &row(2.0));
        });
    }

    #[test]
    fn only_the_changed_lines_are_uploaded_and_the_card_holds_what_was_set() {
        Gpu::for_each_test_mode(|gpu| {
            let mut table = ShapeTable::new(&gpu.alloc, &gpu.queue).unwrap();
            let rows: Vec<u32> = (0..1000).map(|_| table.alloc_row().unwrap()).collect(); // 8 lines of 128 rows (the last one part full)
            for &r in &rows {
                table.set(r, row(r as f32));
            }
            assert_eq!(table.upload(), 8 * BYTES_PER_LINE, "eight lines hold 1000 rows");
            assert_eq!(table.upload(), 0, "nothing changed since");
            for &r in &[0u32, 300, 999] {
                assert_eq!(on_card(gpu, &table, r), row(r as f32).0, "row {r}");
            }
            // One row of line 2 changes: one line goes up, not the table.
            assert!(table.set(300, row(-5.0)));
            assert_eq!(table.upload(), BYTES_PER_LINE);
            assert_eq!(on_card(gpu, &table, 300), row(-5.0).0);
            assert_eq!(on_card(gpu, &table, 299), row(299.0).0, "its neighbours are untouched");
            // Rows in two separate lines: two uploads of one line each (the total counts both).
            table.set(0, row(9.0));
            table.set(900, row(9.0));
            assert_eq!(table.upload(), 2 * BYTES_PER_LINE);
            assert_eq!((on_card(gpu, &table, 0), on_card(gpu, &table, 900)), (row(9.0).0, row(9.0).0));
        });
    }

    #[test]
    fn the_table_grows_when_rows_run_out_keeps_every_row_and_says_so() {
        Gpu::for_each_test_mode(|gpu| {
            let mut table = ShapeTable::new(&gpu.alloc, &gpu.queue).unwrap();
            let first_capacity = table.capacity_rows();
            let before = table.generation();
            let rows: Vec<u32> = (0..first_capacity + 10).map(|_| table.alloc_row().unwrap()).collect();
            assert!(table.capacity_rows() > first_capacity);
            assert_ne!(table.generation(), before, "bind groups must be made again");
            for &r in &rows {
                table.set(r, row((r % 977) as f32));
            }
            table.upload();
            for r in [0, first_capacity - 1, first_capacity, first_capacity + 9] {
                assert_eq!(on_card(gpu, &table, r), row((r % 977) as f32).0, "row {r} after the growth");
            }
        });
    }

    #[test]
    fn a_table_that_cannot_grow_any_more_says_so_instead_of_failing_later() {
        Gpu::for_each_test_mode(|gpu| {
            let alloc = gpu.alloc.clone();
            // Room for 16 lines (the start size) and no more.
            alloc.set_budgets(crate::alloc::Budgets::high().with(Category::ShapeTable, 16 * BYTES_PER_LINE));
            let mut table = ShapeTable::new(&alloc, &gpu.queue).unwrap();
            for _ in 0..table.capacity_rows() {
                table.alloc_row().unwrap();
            }
            let error = table.alloc_row().unwrap_err();
            assert!(matches!(error, AllocError::OverBudget { category: Category::ShapeTable, .. }), "{error}");
            assert_eq!(table.rows_in_use(), table.capacity_rows(), "the failed request took nothing");
            alloc.set_budgets(crate::alloc::Budgets::high());
        });
    }
}
