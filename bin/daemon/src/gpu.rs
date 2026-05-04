// GPU state initialization for wgpu.
// Split into shared GpuContext (device, queue) and per-window WindowSurface.

use std::sync::Arc;
use winit::window::Window;

/// Shared GPU context — device and queue usable across all windows.
pub struct GpuContext {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    instance: wgpu::Instance,
    surface_format: wgpu::TextureFormat,
    present_mode: wgpu::PresentMode,
    alpha_mode: wgpu::CompositeAlphaMode,
}

/// Per-window surface state.
pub struct WindowSurface {
    pub surface: wgpu::Surface<'static>,
    pub config: wgpu::SurfaceConfiguration,
    pub size: (u32, u32),
}

impl GpuContext {
    /// Create GPU context and initial window surface.
    pub async fn new(window: Arc<Window>) -> anyhow::Result<(Self, WindowSurface)> {
        let size = window.inner_size();
        let width = size.width.max(1);
        let height = size.height.max(1);

        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });

        let surface = instance.create_surface(window)?;

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::LowPower,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .ok_or_else(|| anyhow::anyhow!("No suitable GPU adapter found"))?;

        tracing::info!("GPU adapter: {:?}", adapter.get_info().name);

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("cross-notifier"),
                    ..Default::default()
                },
                None,
            )
            .await?;

        let surface_caps = surface.get_capabilities(&adapter);
        let surface_format = surface_caps
            .formats
            .iter()
            .find(|f| f.is_srgb())
            .copied()
            .unwrap_or(surface_caps.formats[0]);

        // Prefer Mailbox (uncapped) with Fifo (vsync) fallback
        let present_mode = if surface_caps
            .present_modes
            .contains(&wgpu::PresentMode::Mailbox)
        {
            wgpu::PresentMode::Mailbox
        } else {
            wgpu::PresentMode::Fifo
        };

        let alpha_mode = pick_alpha_mode(&surface_caps);
        tracing::info!("Alpha mode: {:?}", alpha_mode);

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width,
            height,
            present_mode,
            alpha_mode,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        let ctx = Self {
            device,
            queue,
            instance,
            surface_format,
            present_mode,
            alpha_mode,
        };

        let win_surface = WindowSurface {
            surface,
            config,
            size: (width, height),
        };

        Ok((ctx, win_surface))
    }

    /// Surface format used by this GPU context (needed for pipeline creation).
    pub fn surface_format(&self) -> wgpu::TextureFormat {
        self.surface_format
    }

    /// Create an additional window surface using the existing device and instance.
    pub fn create_surface(&self, window: Arc<Window>) -> anyhow::Result<WindowSurface> {
        let size = window.inner_size();
        let width = size.width.max(1);
        let height = size.height.max(1);

        // Reuse the same instance that created the device
        let surface = self.instance.create_surface(window)?;

        // Query this surface's own capabilities — different windows may support
        // different alpha modes (e.g. the popup supports PreMultiplied but the
        // settings window may only support Opaque/Inherit).
        let adapter = self.instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        });
        let alpha_mode = if let Some(adapter) = pollster::block_on(adapter) {
            let caps = surface.get_capabilities(&adapter);
            pick_alpha_mode(&caps)
        } else {
            self.alpha_mode
        };

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: self.surface_format,
            width,
            height,
            present_mode: self.present_mode,
            alpha_mode,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&self.device, &config);

        Ok(WindowSurface {
            surface,
            config,
            size: (width, height),
        })
    }
}

impl WindowSurface {
    pub fn resize(&mut self, ctx: &GpuContext, width: u32, height: u32) {
        if width > 0 && height > 0 {
            self.size = (width, height);
            self.config.width = width;
            self.config.height = height;
            self.surface.configure(&ctx.device, &self.config);
        }
    }
}

/// Pick the best alpha mode for transparency, preferring premultiplied compositing.
fn pick_alpha_mode(caps: &wgpu::SurfaceCapabilities) -> wgpu::CompositeAlphaMode {
    if caps
        .alpha_modes
        .contains(&wgpu::CompositeAlphaMode::PostMultiplied)
    {
        wgpu::CompositeAlphaMode::PostMultiplied
    } else if caps
        .alpha_modes
        .contains(&wgpu::CompositeAlphaMode::PreMultiplied)
    {
        wgpu::CompositeAlphaMode::PreMultiplied
    } else {
        caps.alpha_modes[0]
    }
}
