//! The one vertex format of the renderer (plan D5, D17, task 2.1) and the triangles of one tessellated shape.
//!
//! Fills and strokes share it. A stroke vertex says where the path is (`point`), which way to push out from it (`normal`,
//! scaled so that pushing by the half width gives the outline, including the extra length of a miter) and how far along the
//! path it is (`distance`, for dashes later). The vertex shader computes `point + normal * half_width`, so the width of a
//! stroke can change on the card (hairlines, width edits) without tessellating again. A fill vertex has a zero normal, so
//! the same formula leaves it where it is. `row` is the row of the shape table that holds the transform, paint and
//! opacity of the draw item the vertex belongs to.

use bytemuck::{Pod, Zeroable};

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct Vertex {
    /// A point of the shape's outline (fill) or of its path (stroke), in the shape's own coordinates.
    pub point: [f32; 2],
    /// Zero for fills. For strokes: the direction to move by the half width to reach the outline.
    pub normal: [f32; 2],
    /// Length of the path up to this vertex (strokes; zero for fills).
    pub distance: f32,
    /// Row of the shape table of the draw item this vertex belongs to.
    pub row: u32,
}

impl Vertex {
    /// Bytes per vertex: 24.
    pub const SIZE: u64 = std::mem::size_of::<Vertex>() as u64;

    /// How the vertex shader sees the vertex buffer.
    pub const LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: Vertex::SIZE,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32, 3 => Uint32],
    };

    /// Where the vertex is for a stroke of the given half width (the formula of the vertex shader).
    pub fn displaced(&self, half_width: f32) -> [f32; 2] {
        [self.point[0] + self.normal[0] * half_width, self.point[1] + self.normal[1] * half_width]
    }
}

/// Triangles: vertices, and `u32` indices into them that start at 0 (the geometry store adds the place where the
/// vertices end up).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Mesh {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
}

impl Mesh {
    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }

    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    /// Bytes this mesh takes on the card.
    pub fn gpu_bytes(&self) -> u64 {
        self.vertices.len() as u64 * Vertex::SIZE + self.indices.len() as u64 * 4
    }

    /// Adds another mesh after this one.
    pub fn append(&mut self, other: Mesh) {
        let offset = self.vertices.len() as u32;
        self.vertices.extend(other.vertices);
        self.indices.extend(other.indices.into_iter().map(|i| i + offset));
    }

    /// Sets the shape-table row of every vertex.
    pub fn set_row(&mut self, row: u32) {
        self.vertices.iter_mut().for_each(|v| v.row = row);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_vertex_is_24_bytes_and_the_layout_matches_the_struct() {
        assert_eq!(Vertex::SIZE, 24);
        assert_eq!(Vertex::LAYOUT.array_stride, 24);
        let attributes = Vertex::LAYOUT.attributes;
        assert_eq!(attributes.len(), 4);
        assert_eq!(attributes.iter().map(|a| a.offset).collect::<Vec<_>>(), [0, 8, 16, 20]);
        assert_eq!(attributes.iter().map(|a| a.shader_location).collect::<Vec<_>>(), [0, 1, 2, 3]);
        assert_eq!(bytemuck::bytes_of(&Vertex { point: [1.0, 2.0], normal: [3.0, 4.0], distance: 5.0, row: 6 }).len(), 24);
    }

    #[test]
    fn displacing_follows_the_normal_by_the_half_width_and_leaves_fills_alone() {
        let stroke = Vertex { point: [10.0, 5.0], normal: [0.0, -1.0], distance: 3.0, row: 0 };
        assert_eq!(stroke.displaced(2.0), [10.0, 3.0]);
        assert_eq!(stroke.displaced(0.0), [10.0, 5.0]);
        let fill = Vertex { point: [1.0, 2.0], normal: [0.0, 0.0], distance: 0.0, row: 0 };
        assert_eq!(fill.displaced(100.0), [1.0, 2.0]);
    }
}
