//! Drawing into an offscreen texture and reading the pixels back.

use crate::alloc::{Category, Tracked};
use crate::device::Gpu;

/// The format of every offscreen picture: 8-bit RGBA, the same layout the CPU renderer's bitmap has.
pub const PICTURE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

impl Gpu {
    /// A texture that can be drawn into and copied out of. Panics if the card cannot give one: this is for tests and
    /// tools, which have no way to carry on without it.
    pub fn picture_texture(&self, width: u32, height: u32) -> Tracked<wgpu::Texture> {
        let texture = self.alloc.create_texture(Category::Other, &wgpu::TextureDescriptor {
            label: Some("picture"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: PICTURE_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        texture.unwrap_or_else(|error| panic!("a {width} x {height} picture texture: {error}"))
    }

    /// Copies `size` bytes of a buffer, starting at `offset` (both multiples of 4), to the CPU. Waits for the card, so it is
    /// for tests and tools. The buffer needs the `COPY_SRC` usage.
    pub fn read_buffer(&self, buffer: &wgpu::Buffer, offset: u64, size: u64) -> Vec<u8> {
        let staging = self
            .alloc
            .create_buffer(
                Category::Other,
                &wgpu::BufferDescriptor { label: Some("buffer readback"), size, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false },
            )
            .unwrap_or_else(|error| panic!("the readback buffer: {error}"));
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("buffer readback") });
        encoder.copy_buffer_to_buffer(buffer, offset, &staging, 0, size);
        self.queue.submit([encoder.finish()]);
        let slice = staging.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        self.device.poll(wgpu::PollType::wait_indefinitely()).expect("the card finished the copy");
        receiver.recv().expect("the map callback ran").expect("the readback buffer could be mapped");
        let bytes = slice.get_mapped_range().expect("the mapped readback buffer can be read").to_vec();
        staging.unmap();
        bytes
    }

    /// Copies a rectangle of any (single-sample, color) texture to the CPU, rows top to bottom with no padding. Waits for
    /// the card, so it is for tests and tools.
    pub fn read_texture_part(&self, texture: &wgpu::Texture, x: u32, y: u32, width: u32, height: u32) -> Vec<u8> {
        let texel = texture.format().block_copy_size(None).expect("a color format");
        let row_bytes = width * texel;
        let padded_row_bytes = row_bytes.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = self
            .alloc
            .create_buffer(
                Category::Other,
                &wgpu::BufferDescriptor {
                    label: Some("texture readback"),
                    size: (padded_row_bytes * height) as u64,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                },
            )
            .unwrap_or_else(|error| panic!("the readback buffer of a {width} x {height} part: {error}"));
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("texture readback") });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture, mip_level: 0, origin: wgpu::Origin3d { x, y, z: 0 }, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(padded_row_bytes), rows_per_image: Some(height) },
            },
            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        );
        self.queue.submit([encoder.finish()]);
        let slice = buffer.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        self.device.poll(wgpu::PollType::wait_indefinitely()).expect("the card finished the copy");
        receiver.recv().expect("the map callback ran").expect("the readback buffer could be mapped");
        let padded = slice.get_mapped_range().expect("the mapped readback buffer can be read");
        let mut bytes = Vec::with_capacity((row_bytes * height) as usize);
        for row in padded.chunks_exact(padded_row_bytes as usize) {
            bytes.extend_from_slice(&row[..row_bytes as usize]);
        }
        drop(padded);
        buffer.unmap();
        bytes
    }

    /// Copies a finished picture to the CPU: `width * height * 4` bytes, rows top to bottom, no padding.
    /// Waits for the card to finish, so it is for tests and tools, not for a frame loop.
    pub fn read_picture(&self, texture: &wgpu::Texture) -> Vec<u8> {
        let (width, height) = (texture.width(), texture.height());
        // Rows in the copy must start on a 256-byte boundary.
        let row_bytes = width * 4;
        let padded_row_bytes = row_bytes.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = self
            .alloc
            .create_buffer(
                Category::Other,
                &wgpu::BufferDescriptor {
                    label: Some("picture readback"),
                    size: (padded_row_bytes * height) as u64,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                },
            )
            .unwrap_or_else(|error| panic!("the readback buffer of a {width} x {height} picture: {error}"));

        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("picture readback") });
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(padded_row_bytes), rows_per_image: Some(height) },
            },
            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        );
        self.queue.submit([encoder.finish()]);

        let slice = buffer.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        self.device.poll(wgpu::PollType::wait_indefinitely()).expect("the card finished the copy");
        receiver.recv().expect("the map callback ran").expect("the readback buffer could be mapped");

        let padded = slice.get_mapped_range().expect("the mapped readback buffer can be read");
        let mut pixels = Vec::with_capacity((row_bytes * height) as usize);
        for row in padded.chunks_exact(padded_row_bytes as usize) {
            pixels.extend_from_slice(&row[..row_bytes as usize]);
        }
        drop(padded);
        buffer.unmap();
        pixels
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Clears `texture` to `color` with the card, the smallest possible "drawing".
    fn clear(gpu: &Gpu, texture: &wgpu::Texture, color: wgpu::Color) {
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("clear"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(color), store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        gpu.queue.submit([encoder.finish()]);
    }

    #[test]
    fn a_cleared_picture_reads_back_with_the_color_it_was_cleared_to() {
        Gpu::for_each_test_mode(|gpu| {
            // 70 pixels wide: 280 bytes per row is not a multiple of 256, so this exercises the row padding.
            let texture = gpu.picture_texture(70, 5);
            clear(gpu, &texture, wgpu::Color { r: 1.0, g: 0.0, b: 0.0, a: 1.0 });
            let pixels = gpu.read_picture(&texture);
            assert_eq!(pixels.len(), 70 * 5 * 4);
            assert!(pixels.as_chunks::<4>().0.iter().all(|p| *p == [255, 0, 0, 255]), "first pixel {:?}", &pixels[..4]);

            clear(gpu, &texture, wgpu::Color { r: 0.0, g: 0.0, b: 1.0, a: 1.0 });
            assert!(gpu.read_picture(&texture).as_chunks::<4>().0.iter().all(|p| *p == [0, 0, 255, 255]), "a second picture reads back too");
        });
    }
}
