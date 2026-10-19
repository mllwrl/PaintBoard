use std::sync::{Arc, Mutex};

use vello::peniko::Color;
use vello::util::{RenderContext, RenderSurface};
use vello::wgpu::{self, CurrentSurfaceTexture, SurfaceTarget};
use vello::{AaConfig, AaSupport, RenderParams, Renderer, RendererOptions, Scene};

/// What a pass drawn on top of the canvas (the UI) gets to work with.
pub struct FrameTarget<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub encoder: &'a mut wgpu::CommandEncoder,
    /// The surface texture being presented; the canvas is already in it.
    pub view: &'a wgpu::TextureView,
    /// Size in physical pixels.
    pub size: [u32; 2],
}

/// GPU device, window surface and the vello renderer.
pub struct Gpu {
    context: RenderContext,
    surface: RenderSurface<'static>,
    renderer: Renderer,
}

impl Gpu {
    /// Async because adapter and device requests cannot block on the web.
    pub async fn new(
        target: impl Into<SurfaceTarget<'static>>,
        width: u32,
        height: u32,
    ) -> Result<Self, vello::Error> {
        let mut context = RenderContext::new();
        let surface = context
            .create_surface(
                target,
                width.max(1),
                height.max(1),
                wgpu::PresentMode::AutoVsync,
            )
            .await?;
        let device = &context.devices[surface.dev_id].device;
        let renderer = Renderer::new(
            device,
            RendererOptions {
                antialiasing_support: AaSupport::area_only(),
                ..Default::default()
            },
        )?;
        Ok(Self {
            context,
            surface,
            renderer,
        })
    }

    pub fn device(&self) -> &wgpu::Device {
        &self.context.devices[self.surface.dev_id].device
    }

    /// The graphics card and API in use, for the log.
    pub fn describe(&self) -> String {
        let info = self.context.devices[self.surface.dev_id]
            .adapter()
            .get_info();
        format!("{} ({:?})", info.name, info.backend)
    }

    /// Format of the window surface, for renderers drawing into it.
    pub fn surface_format(&self) -> wgpu::TextureFormat {
        self.surface.config.format
    }

    /// The largest image side [`Gpu::render_image`] can draw at once.
    pub fn max_image_side(&self) -> u32 {
        self.device().limits().max_texture_dimension_2d
    }

    /// Like [`Gpu::render_image`], without waiting: the pixels come back later (the only
    /// way on the web). Call [`Gpu::poll`] until [`PendingImage::is_ready`].
    pub fn start_image(
        &mut self,
        scene: &Scene,
        width: u32,
        height: u32,
        background: Color,
    ) -> Result<PendingImage, String> {
        let handle = &self.context.devices[self.surface.dev_id];
        start_pixels(
            handle,
            &mut self.renderer,
            scene,
            (width, height),
            background,
        )
    }

    /// Lets finished GPU work report back; `wait` blocks until all of it is done (not on
    /// the web, where the browser reports on its own).
    pub fn poll(&self, wait: bool) {
        let poll = if wait && !cfg!(target_arch = "wasm32") {
            wgpu::PollType::wait_indefinitely()
        } else {
            wgpu::PollType::Poll
        };
        if let Err(err) = self.device().poll(poll) {
            log::warn!("GPU poll failed: {err}");
        }
    }

    /// Draws `scene` off screen into a `width`×`height` image and reads it back as rows of
    /// straight (not premultiplied) RGBA8 pixels. Blocks until the GPU is done; on the web,
    /// where it cannot wait, it fails: use [`Gpu::start_image`] there.
    pub fn render_image(
        &mut self,
        scene: &Scene,
        width: u32,
        height: u32,
        background: Color,
    ) -> Result<Vec<u8>, String> {
        let handle = &self.context.devices[self.surface.dev_id];
        render_pixels(
            handle,
            &mut self.renderer,
            scene,
            (width, height),
            background,
        )
    }
}

/// A GPU without a window, for drawing scenes into images (tests, tools).
pub struct Offscreen {
    context: RenderContext,
    dev_id: usize,
    renderer: Renderer,
}

impl Offscreen {
    /// `None` when there is no usable GPU (e.g. on a build server).
    pub async fn new() -> Option<Self> {
        let mut context = RenderContext::new();
        let dev_id = context.device(None).await?;
        let renderer = Renderer::new(
            &context.devices[dev_id].device,
            RendererOptions {
                antialiasing_support: AaSupport::area_only(),
                ..Default::default()
            },
        )
        .ok()?;
        Some(Self {
            context,
            dev_id,
            renderer,
        })
    }

    /// Like [`Gpu::render_image`].
    pub fn render_image(
        &mut self,
        scene: &Scene,
        width: u32,
        height: u32,
        background: Color,
    ) -> Result<Vec<u8>, String> {
        let handle = &self.context.devices[self.dev_id];
        render_pixels(
            handle,
            &mut self.renderer,
            scene,
            (width, height),
            background,
        )
    }
}

/// Draws `scene` into a new texture of `size` and reads it back as straight RGBA8 rows,
/// waiting for the GPU (not possible on the web).
fn render_pixels(
    handle: &vello::util::DeviceHandle,
    renderer: &mut Renderer,
    scene: &Scene,
    size: (u32, u32),
    background: Color,
) -> Result<Vec<u8>, String> {
    let pending = start_pixels(handle, renderer, scene, size, background)?;
    handle
        .device
        .poll(wgpu::PollType::wait_indefinitely())
        .map_err(|err| err.to_string())?;
    pending.take()
}

/// Pixels on their way back from the GPU; see [`Gpu::start_image`].
pub struct PendingImage {
    buffer: wgpu::Buffer,
    /// Bytes in a row of the image, and in a row of the buffer (aligned for the copy).
    row: u32,
    padded: u32,
    /// Set by the GPU once the buffer can be read.
    mapped: Arc<Mutex<Option<Result<(), wgpu::BufferAsyncError>>>>,
}

impl PendingImage {
    /// The pixels can be taken.
    pub fn is_ready(&self) -> bool {
        self.mapped.lock().is_ok_and(|m| m.is_some())
    }

    /// The pixels, as rows of straight RGBA8; fails if not ready yet.
    pub fn take(self) -> Result<Vec<u8>, String> {
        let outcome = self.mapped.lock().map_err(|e| e.to_string())?.take();
        match outcome {
            Some(Ok(())) => {}
            Some(Err(err)) => return Err(err.to_string()),
            None => return Err("the GPU did not finish drawing".to_owned()),
        }
        let slice = self.buffer.slice(..);
        let mapped = slice.get_mapped_range().map_err(|err| err.to_string())?;
        let mut pixels =
            Vec::with_capacity(mapped.len() / self.padded as usize * self.row as usize);
        for line in mapped.chunks(self.padded as usize) {
            pixels.extend_from_slice(&line[..self.row as usize]);
        }
        drop(mapped);
        self.buffer.unmap();
        Ok(pixels)
    }
}

/// Draws `scene` into a new texture of `size` and starts reading it back.
fn start_pixels(
    handle: &vello::util::DeviceHandle,
    renderer: &mut Renderer,
    scene: &Scene,
    (width, height): (u32, u32),
    background: Color,
) -> Result<PendingImage, String> {
    let (device, queue) = (&handle.device, &handle.queue);
    let size = wgpu::Extent3d {
        width,
        height,
        depth_or_array_layers: 1,
    };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("export"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let params = RenderParams {
        base_color: background,
        width,
        height,
        antialiasing_method: AaConfig::Area,
    };
    renderer
        .render_to_texture(device, queue, scene, &view, &params)
        .map_err(|err| err.to_string())?;

    let row = width * 4;
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let padded = row.div_ceil(align) * align;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("export readback"),
        size: u64::from(padded) * u64::from(height),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("export readback"),
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: None,
            },
        },
        size,
    );
    queue.submit([encoder.finish()]);
    let mapped = Arc::new(Mutex::new(None));
    let sink = mapped.clone();
    buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            if let Ok(mut slot) = sink.lock() {
                *slot = Some(result);
            }
        });
    Ok(PendingImage {
        buffer,
        row,
        padded,
        mapped,
    })
}

impl Gpu {
    pub fn resize(&mut self, width: u32, height: u32) {
        // A minimized window reports a zero size, which wgpu cannot configure.
        if width == 0 || height == 0 {
            return;
        }
        self.context
            .resize_surface(&mut self.surface, width, height);
    }

    /// Draws `scene`, then lets `on_top` add passes to the same frame before it is shown.
    /// `on_top` is not called when no frame could be acquired.
    pub fn render(
        &mut self,
        scene: &Scene,
        background: Color,
        on_top: impl FnOnce(FrameTarget<'_>),
    ) {
        let (frame, suboptimal) = match self.surface.surface.get_current_texture() {
            CurrentSurfaceTexture::Success(frame) => (frame, false),
            CurrentSurfaceTexture::Suboptimal(frame) => (frame, true),
            CurrentSurfaceTexture::Outdated => {
                self.context.configure_surface(&self.surface);
                return;
            }
            CurrentSurfaceTexture::Lost => {
                // TODO: recreate the surface; reconfiguring is enough for most resize-related losses.
                log::warn!("surface lost, reconfiguring");
                self.context.configure_surface(&self.surface);
                return;
            }
            CurrentSurfaceTexture::Timeout
            | CurrentSurfaceTexture::Occluded
            | CurrentSurfaceTexture::Validation => return,
        };

        let handle = &self.context.devices[self.surface.dev_id];
        let params = RenderParams {
            base_color: background,
            width: self.surface.config.width,
            height: self.surface.config.height,
            antialiasing_method: AaConfig::Area,
        };
        if let Err(err) = self.renderer.render_to_texture(
            &handle.device,
            &handle.queue,
            scene,
            &self.surface.target_view,
            &params,
        ) {
            log::error!("render failed: {err}");
            return;
        }

        let mut encoder = handle
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("blit to surface"),
            });
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        self.surface.blitter.copy(
            &handle.device,
            &mut encoder,
            &self.surface.target_view,
            &view,
        );
        on_top(FrameTarget {
            device: &handle.device,
            queue: &handle.queue,
            encoder: &mut encoder,
            view: &view,
            size: [self.surface.config.width, self.surface.config.height],
        });
        handle.queue.submit([encoder.finish()]);
        handle.queue.present(frame);

        if suboptimal {
            self.context.configure_surface(&self.surface);
        }
    }
}
