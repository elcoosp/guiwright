//! The §4.1 pixel path (m77): headless wgpu offscreen render + buffer
//! readback — the `egui_kittest` shape, no surface required.
//!
//! The adapter tessellates the pass exactly like the digest path does, but
//! instead of hashing geometry it renders the `ClippedPrimitive`s through
//! [`egui_wgpu::Renderer`] into an offscreen `RGBA8UnormSrgb` texture,
//! copies the texture into a map-able buffer (256-byte row alignment
//! stripped), and hands the premultiplied RGBA8 frame to the §15.5
//! capture pipeline. Frames render **once per egui pass**, keyed by
//! [`egui::Context::cumulative_pass_nr`] — video capture, `app.screenshot`
//! and `locator.screenshot` share the cache (§15.4 render-once).
//!
//! GPU-less hosts answer [`AdapterError::Unsupported`] (the masonry
//! precedent): the probe happens once at the first pixel demand, and the
//! failure is cached — a headless CI box without lavapipe never retries.

use std::sync::Arc;

use egui::epaint::ClippedPrimitive;

/// A rendered frame and the pass it belongs to.
struct CachedFrame {
    pass_nr: u64,
    size: (u32, u32),
    rgba: Arc<[u8]>,
}

/// The headless wgpu context. Created lazily; the probe's outcome is
/// cached so a GPU-less host pays the adapter enumeration once.
pub struct PixelBackend {
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: egui_wgpu::Renderer,
    target: wgpu::Texture,
    size: (u32, u32),
    cached: Option<CachedFrame>,
}

/// Row stride alignment for `copy_texture_to_buffer` (256 bytes).
fn padded_bytes_per_row(width: u32) -> u32 {
    let unpadded = width * 4;
    unpadded.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT
}

impl PixelBackend {
    /// Probe for a headless adapter (no compatible surface — §11
    /// "windowless"). The failure mode is a string, not a panic: a
    /// GPU-less host is a supported configuration (the graceful row).
    pub fn probe() -> Result<Self, String> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
                .map_err(|e| format!("no wgpu adapter for headless pixels: {e}"))?;
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
                .map_err(|e| format!("no wgpu device for headless pixels: {e}"))?;
        let renderer = egui_wgpu::Renderer::new(
            &device,
            wgpu::TextureFormat::Rgba8UnormSrgb,
            egui_wgpu::RendererOptions::default(),
        );
        let size = (0, 0);
        let target = Self::create_target(&device, size);
        Ok(Self {
            device,
            queue,
            renderer,
            target,
            size,
            cached: None,
        })
    }

    fn create_target(device: &wgpu::Device, size: (u32, u32)) -> wgpu::Texture {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("guiwright-egui-offscreen"),
            size: wgpu::Extent3d {
                width: size.0.max(1),
                height: size.1.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        })
    }

    /// Render the pass's primitives into the offscreen target and read the
    /// buffer back. `textures_delta` must be this pass's (fonts ride it).
    pub fn render(
        &mut self,
        textures_delta: &egui::TexturesDelta,
        paint_jobs: &[ClippedPrimitive],
        size: (u32, u32),
        pixels_per_point: f32,
    ) -> Result<Arc<[u8]>, String> {
        for (id, deltas) in &textures_delta.set {
            for delta in deltas {
                self.renderer
                    .update_texture(&self.device, &self.queue, *id, delta);
            }
        }
        for id in &textures_delta.free {
            self.renderer.free_texture(id);
        }

        if self.size != size {
            self.target = Self::create_target(&self.device, size);
            self.size = size;
        }

        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [size.0, size.1],
            pixels_per_point,
        };
        let target_view = self
            .target
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("guiwright-egui-pixels"),
            });
        self.renderer
            .update_buffers(&self.device, &self.queue, &mut encoder, paint_jobs, &screen);
        {
            let render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("guiwright-egui-offscreen-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target_view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            // The pass keeps its referenced resources alive; the lifetime
            // is erased so the encoder can finish inside this scope.
            self.renderer
                .render(&mut render_pass.forget_lifetime(), paint_jobs, &screen);
        }

        // Readback: copy the target into a map-able buffer, strip the row
        // padding, and hand the premultiplied RGBA8 frame over.
        let bytes_per_row = padded_bytes_per_row(size.0.max(1));
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("guiwright-egui-readback"),
            size: u64::from(bytes_per_row) * u64::from(size.1.max(1)),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            self.target.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: None,
                },
            },
            wgpu::Extent3d {
                width: size.0.max(1),
                height: size.1.max(1),
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit([encoder.finish()]);

        let slice = buffer.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        // Wait covers the most recent submission — this one.
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
        receiver
            .recv()
            .map_err(|e| format!("readback mapping dropped: {e}"))?
            .map_err(|e| format!("readback mapping failed: {e}"))?;

        let data = slice
            .get_mapped_range()
            .map_err(|e| format!("readback map failed: {e}"))?;
        let row = usize::try_from(bytes_per_row).unwrap_or(0);
        let width = usize::try_from(size.0.max(1)).unwrap_or(0);
        let height = usize::try_from(size.1.max(1)).unwrap_or(0);
        let mut rgba = Vec::with_capacity(width * height * 4);
        for row_index in 0..height {
            let start = row_index * row;
            rgba.extend_from_slice(&data[start..start + width * 4]);
        }
        drop(data);
        buffer.unmap();

        Ok(Arc::from(rgba))
    }

    /// The cached frame for `pass_nr`, if this pass already rendered.
    pub fn cached(&self, pass_nr: u64, size: (u32, u32)) -> Option<Arc<[u8]>> {
        let frame = self.cached.as_ref()?;
        (frame.pass_nr == pass_nr && frame.size == size).then(|| Arc::clone(&frame.rgba))
    }

    pub fn remember(&mut self, pass_nr: u64, size: (u32, u32), rgba: Arc<[u8]>) {
        self.cached = Some(CachedFrame {
            pass_nr,
            size,
            rgba,
        });
    }
}
