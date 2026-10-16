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

    /// Format of the window surface, for renderers drawing into it.
    pub fn surface_format(&self) -> wgpu::TextureFormat {
        self.surface.config.format
    }

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
