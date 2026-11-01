use super::{Offscreen, RenderDeadline, Renderer};
use crate::contact_texture::{inert_field_texture, white_base};
use crate::error::RenderError;
use std::sync::mpsc;
use wgpu::TextureView;

pub(super) struct RenderTargets<'a> {
    pub(super) color: &'a TextureView,
    pub(super) depth: &'a TextureView,
}

/// The offscreen path's 1x1 white group-2 binding, for meshes with no material
/// texture. Same pixel and same layout as [`crate::texture::GpuTexture::fallback`],
/// which the live path uses; that one also keeps the texture and sampler around,
/// which nothing here needs.
///
/// Binding 2 is the inert contact field: a single sentinel texel. Wgpu requires
/// every binding in the layout to be present, and a scan rendered through this
/// group paints no contacts (`contact_map == 0`), so the value is never read.
pub(super) fn make_fallback_texture_bind_group(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &Renderer,
) -> wgpu::BindGroup {
    // The returned textures are not kept: the bind group holds a
    // reference to each view, and a view keeps its texture alive.
    let (_base_texture, tex_view, sampler) = white_base(device, queue);
    let (_field_texture, field_view) = inert_field_texture(device, queue);
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("occluview fallback texture bind group"),
        layout: renderer.texture_layout(),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&tex_view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(&field_view),
            },
        ],
    })
}

pub(super) fn make_color_target(device: &wgpu::Device, size: u32) -> (wgpu::Texture, TextureView) {
    make_color_target_extent(device, size, size)
}

pub(super) fn make_color_target_extent(
    device: &wgpu::Device,
    width: u32,
    height: u32,
) -> (wgpu::Texture, TextureView) {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("occluview offscreen color"),
        size: extent_rect(width, height),
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    (tex, view)
}

pub(super) fn make_depth_target(
    device: &wgpu::Device,
    size: u32,
    format: wgpu::TextureFormat,
) -> (wgpu::Texture, TextureView) {
    make_depth_target_extent(device, size, size, format)
}

pub(super) fn make_depth_target_extent(
    device: &wgpu::Device,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
) -> (wgpu::Texture, TextureView) {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("occluview offscreen depth"),
        size: extent_rect(width, height),
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    (tex, view)
}

pub(super) fn extent(size: u32) -> wgpu::Extent3d {
    extent_rect(size, size)
}

pub(super) fn extent_rect(width: u32, height: u32) -> wgpu::Extent3d {
    wgpu::Extent3d {
        width,
        height,
        depth_or_array_layers: 1,
    }
}

pub(super) fn is_transparent(opacity: f32) -> bool {
    opacity < 0.999
}

pub(super) fn padded_bytes_per_row(width: u32) -> u32 {
    let unpadded = width * 4;
    unpadded.div_ceil(256) * 256
}

fn readback_timeout(deadline: RenderDeadline) -> RenderError {
    deadline.timeout_error()
}

fn wait_for_map_callback(
    map_rx: &mpsc::Receiver<Result<(), String>>,
    deadline: RenderDeadline,
) -> Result<(), RenderError> {
    let mapped = match map_rx.try_recv() {
        Ok(mapped) => mapped,
        Err(mpsc::TryRecvError::Empty) => match deadline.poll_timeout()? {
            Some(timeout) => map_rx.recv_timeout(timeout).map_err(|error| match error {
                mpsc::RecvTimeoutError::Timeout => readback_timeout(deadline),
                mpsc::RecvTimeoutError::Disconnected => {
                    RenderError::Surface("offscreen readback callback dropped".to_owned())
                }
            })?,
            None => map_rx.recv().map_err(|_| {
                RenderError::Surface("offscreen readback callback dropped".to_owned())
            })?,
        },
        Err(mpsc::TryRecvError::Disconnected) => {
            return Err(RenderError::Surface(
                "offscreen readback callback dropped".to_owned(),
            ));
        }
    };

    mapped.map_err(|error| RenderError::Surface(format!("offscreen readback failed: {error}")))
}

impl Offscreen {
    pub(super) fn read_back(
        &self,
        output_buffer: &wgpu::Buffer,
        padded_bytes_per_row: u32,
        size_px: u16,
        deadline: RenderDeadline,
    ) -> Result<Vec<u8>, RenderError> {
        self.read_back_extent(
            output_buffer,
            padded_bytes_per_row,
            [size_px, size_px],
            deadline,
        )
    }

    pub(super) fn read_back_extent(
        &self,
        output_buffer: &wgpu::Buffer,
        padded_bytes_per_row: u32,
        size_px: [u16; 2],
        deadline: RenderDeadline,
    ) -> Result<Vec<u8>, RenderError> {
        let [width_px, height_px] = size_px;
        let slice = output_buffer.slice(..);
        let (map_tx, map_rx) = mpsc::sync_channel(1);
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = map_tx.send(result.map_err(|error| error.to_string()));
        });
        let poll_timeout = deadline.poll_timeout()?;
        let poll_result = self.renderer.device().poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: poll_timeout,
        });
        // Every offscreen frame -- thumbnail, preview, cut view -- lands here
        // after its submit, and the bounded wait above is the point by which the
        // driver has reported anything it refused. The device's error handler
        // records rather than panics (a panic in the shell surrogate is a
        // crash), so nothing else asks. Unasked, a refused buffer allocation
        // or a lost device would produce a frame of zeroes that the caller
        // returns as a valid transparent thumbnail -- which Explorer then
        // caches against the file's timestamp and never recomputes.
        if let Some(error) = self.renderer.take_gpu_error() {
            output_buffer.unmap();
            return Err(RenderError::Surface(error));
        }
        if self.renderer.is_gpu_faulted() {
            output_buffer.unmap();
            return Err(RenderError::Surface(
                "offscreen GPU renderer became unavailable during readback".to_owned(),
            ));
        }
        let mapped = match poll_result {
            Ok(_) => wait_for_map_callback(&map_rx, deadline),
            Err(wgpu::PollError::Timeout) => Err(readback_timeout(deadline)),
            Err(error) => Err(RenderError::Surface(format!(
                "offscreen readback poll failed: {error}"
            ))),
        };
        if let Err(error) = mapped {
            // `unmap` cancels an outstanding map operation, so the local
            // readback buffer is not dropped while wgpu still considers it
            // mapped or pending.
            output_buffer.unmap();
            return Err(error);
        }

        let row_bytes = usize::from(width_px) * 4;
        let row_count = usize::from(height_px);
        let pixels_result = (|| {
            let data = match slice.get_mapped_range() {
                Ok(data) => data,
                Err(error) => return Err(RenderError::Surface(error.to_string())),
            };
            let mut out = Vec::with_capacity(row_bytes * row_count);
            for row in 0..row_count {
                let start = row * padded_bytes_per_row as usize;
                out.extend_from_slice(&data[start..start + row_bytes]);
            }
            Ok(out)
        })();
        output_buffer.unmap();
        let pixels = pixels_result?;

        // Rows come back in framebuffer order, which in wgpu is top-down: NDC
        // +y is the first row, and this is the convention every consumer here
        // already documents — egui paints the buffer untouched, the section
        // ruler's `SlicePlaneMap` puts +up at the top, `pixels_to_hbitmap`
        // builds a top-down DIB, and the CLI writes a PNG in row order.
        // Reversing here would make the data bottom-up and mirror the
        // Explorer thumbnail, the CLI's PNG and the fallback viewport.
        Ok(pixels)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn pending_map_callback_returns_a_structured_deadline_error() {
        let (_map_tx, map_rx) = mpsc::sync_channel(1);

        assert!(matches!(
            wait_for_map_callback(&map_rx, RenderDeadline::at(Instant::now())),
            Err(RenderError::ReadbackTimeout { timeout }) if timeout == Duration::ZERO
        ));
    }

    #[test]
    fn caller_owned_deadline_expires_without_a_global_readback_budget() {
        let expired_at = Instant::now()
            .checked_sub(Duration::from_secs(1))
            .expect("one second before now is representable");
        let deadline = RenderDeadline::at(expired_at);

        assert!(matches!(
            deadline.remaining(),
            Err(RenderError::ReadbackTimeout {
                timeout: Duration::ZERO
            })
        ));
    }

    #[test]
    fn map_callback_accepts_the_callers_absolute_deadline() {
        let (_map_tx, map_rx) = mpsc::sync_channel(1);

        assert!(matches!(
            wait_for_map_callback(&map_rx, RenderDeadline::at(Instant::now())),
            Err(RenderError::ReadbackTimeout {
                timeout: Duration::ZERO
            })
        ));
    }

    #[test]
    fn completed_map_callback_allows_readback() {
        let (map_tx, map_rx) = mpsc::sync_channel(1);
        assert!(map_tx.send(Ok(())).is_ok(), "test sender stays connected");

        assert!(
            wait_for_map_callback(&map_rx, RenderDeadline::after(Duration::from_secs(1))).is_ok(),
            "a completed map must preserve the normal readback path"
        );
    }

    #[test]
    fn failed_map_callback_reports_the_driver_error() {
        let (map_tx, map_rx) = mpsc::sync_channel(1);
        assert!(
            map_tx.send(Err("device lost".to_owned())).is_ok(),
            "test sender stays connected"
        );

        assert!(matches!(
            wait_for_map_callback(&map_rx, RenderDeadline::after(Duration::from_secs(1))),
            Err(RenderError::Surface(error)) if error == "offscreen readback failed: device lost"
        ));
    }

    #[test]
    fn disconnected_map_callback_reports_a_surface_error() {
        let (map_tx, map_rx) = mpsc::sync_channel::<Result<(), String>>(1);
        drop(map_tx);

        assert!(matches!(
            wait_for_map_callback(&map_rx, RenderDeadline::after(Duration::from_secs(1))),
            Err(RenderError::Surface(error)) if error == "offscreen readback callback dropped"
        ));
    }
}
