use std::collections::BTreeSet;
use std::ffi::{CStr, CString};
use std::io::Cursor;
use std::os::raw::c_void;

use ash::{Device, Entry, Instance, vk};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::{debug, error, info, trace, warn};
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};

const VALIDATION_LAYER: &CStr = c"VK_LAYER_KHRONOS_validation";
const WINDOW_TITLE: &str = "FinalEngine Vulkan Renderer";
const DEFAULT_WIDTH: u32 = 1280;
const DEFAULT_HEIGHT: u32 = 720;
const TRIANGLE_VERTEX_SHADER: &[u8] = include_bytes!("../shaders/triangle.vert.spv");
const TRIANGLE_FRAGMENT_SHADER: &[u8] = include_bytes!("../shaders/triangle.frag.spv");

#[derive(Debug, Error)]
pub enum RenderError {
    #[error("failed to load Vulkan entry points: {0}")]
    LoadVulkan(#[from] ash::LoadingError),
    #[error("Vulkan call failed: {0:?}")]
    Vulkan(#[from] vk::Result),
    #[error("window handle error: {0}")]
    WindowHandle(#[from] raw_window_handle::HandleError),
    #[error("winit event loop error: {0}")]
    EventLoop(#[from] winit::error::EventLoopError),
    #[error("winit OS error: {0}")]
    Os(#[from] winit::error::OsError),
    #[error("{0}")]
    Message(String),
}

pub type RenderResult<T> = Result<T, RenderError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RenderBackend {
    Vulkan,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct VulkanFeaturePolicy {
    pub require_vulkan_1_3: bool,
    pub prefer_ray_tracing: bool,
    pub allow_raster_fallback: bool,
    pub enable_validation_layers: bool,
}

impl Default for VulkanFeaturePolicy {
    fn default() -> Self {
        Self {
            require_vulkan_1_3: true,
            prefer_ray_tracing: true,
            allow_raster_fallback: true,
            enable_validation_layers: cfg!(debug_assertions),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RendererPath {
    Raster,
    RayTracing,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RendererConfig {
    pub backend: RenderBackend,
    pub features: VulkanFeaturePolicy,
    pub shader_source: ShaderSource,
}

impl RendererConfig {
    pub fn vulkan_high_end() -> Self {
        Self {
            backend: RenderBackend::Vulkan,
            features: VulkanFeaturePolicy::default(),
            shader_source: ShaderSource::Slang,
        }
    }

    pub fn choose_path(&self, ray_tracing_available: bool) -> RendererPath {
        if self.features.prefer_ray_tracing && ray_tracing_available {
            RendererPath::RayTracing
        } else {
            RendererPath::Raster
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShaderSource {
    Slang,
}

#[derive(Debug, Clone)]
pub struct VulkanWindowConfig {
    pub title: String,
    pub width: u32,
    pub height: u32,
    pub clear_color: [f32; 4],
    pub max_frames: Option<u64>,
}

impl Default for VulkanWindowConfig {
    fn default() -> Self {
        Self {
            title: WINDOW_TITLE.to_string(),
            width: DEFAULT_WIDTH,
            height: DEFAULT_HEIGHT,
            clear_color: [0.02, 0.04, 0.08, 1.0],
            max_frames: None,
        }
    }
}

pub fn run_vulkan_renderer(config: RendererConfig) -> RenderResult<()> {
    run_vulkan_renderer_with_window_config(config, VulkanWindowConfig::default())
}

pub fn run_vulkan_renderer_with_window_config(
    config: RendererConfig,
    window_config: VulkanWindowConfig,
) -> RenderResult<()> {
    info!("Starting FinalEngine Vulkan renderer");
    info!("Renderer config: {config:#?}");
    info!("Window config: {window_config:#?}");
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(winit::event_loop::ControlFlow::Poll);

    let mut app = VulkanApp::new(config, window_config);
    event_loop.run_app(&mut app)?;

    if let Some(error) = app.fatal_error {
        return Err(error);
    }

    info!("Vulkan renderer shut down cleanly");
    Ok(())
}

struct VulkanApp {
    renderer_config: RendererConfig,
    window_config: VulkanWindowConfig,
    renderer: Option<VulkanRenderer>,
    fatal_error: Option<RenderError>,
    presented_frames: u64,
}

impl VulkanApp {
    fn new(renderer_config: RendererConfig, window_config: VulkanWindowConfig) -> Self {
        Self {
            renderer_config,
            window_config,
            renderer: None,
            fatal_error: None,
            presented_frames: 0,
        }
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, error: RenderError) {
        error!("Fatal renderer error: {error}");
        self.fatal_error = Some(error);
        event_loop.exit();
    }
}

impl ApplicationHandler for VulkanApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        info!("winit resumed; creating Vulkan window and renderer");
        if self.renderer.is_some() {
            info!("Renderer already exists after resume; no new renderer needed");
            return;
        }

        let window_attributes = Window::default_attributes()
            .with_title(self.window_config.title.clone())
            .with_inner_size(PhysicalSize::new(
                self.window_config.width,
                self.window_config.height,
            ));

        let window = match event_loop.create_window(window_attributes) {
            Ok(window) => window,
            Err(error) => return self.fail(event_loop, error.into()),
        };

        match VulkanRenderer::new(
            window,
            self.renderer_config.clone(),
            self.window_config.clone(),
        ) {
            Ok(renderer) => {
                info!("Vulkan renderer is ready; requesting first redraw");
                renderer.window.request_redraw();
                self.renderer = Some(renderer);
            }
            Err(error) => self.fail(event_loop, error),
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };

        if renderer.window.id() != window_id {
            warn!("Ignoring event for unknown window {window_id:?}");
            return;
        }

        match event {
            WindowEvent::CloseRequested => {
                info!("Close requested; exiting renderer loop");
                event_loop.exit();
            }
            WindowEvent::KeyboardInput { event, .. }
                if event.state.is_pressed()
                    && matches!(event.logical_key, Key::Named(NamedKey::Escape)) =>
            {
                info!("Escape pressed; exiting renderer loop");
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                info!("Window resized to {}x{}", size.width, size.height);
                renderer.resize(size);
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                info!("Window scale factor changed to {scale_factor}");
                renderer.mark_swapchain_dirty();
            }
            WindowEvent::RedrawRequested => {
                if let Err(error) = renderer.draw_frame() {
                    self.fail(event_loop, error);
                    return;
                }
                self.presented_frames += 1;
                if let Some(max_frames) = self.window_config.max_frames
                    && self.presented_frames >= max_frames
                {
                    info!(
                        "Reached configured max frame count ({max_frames}); exiting renderer loop"
                    );
                    event_loop.exit();
                    return;
                }
                renderer.window.request_redraw();
            }
            WindowEvent::Occluded(occluded) => {
                info!("Window occlusion changed: {occluded}");
                renderer.occluded = occluded;
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(renderer) = self.renderer.as_ref() {
            renderer.window.request_redraw();
        }
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        info!("winit exiting; dropping Vulkan renderer");
        self.renderer.take();
    }
}

struct VulkanRenderer {
    window: Window,
    _entry: Entry,
    instance: Instance,
    debug_utils: Option<ash::ext::debug_utils::Instance>,
    debug_messenger: Option<vk::DebugUtilsMessengerEXT>,
    surface_loader: ash::khr::surface::Instance,
    surface: vk::SurfaceKHR,
    physical_device: vk::PhysicalDevice,
    device: Device,
    graphics_queue: vk::Queue,
    present_queue: vk::Queue,
    queue_family_indices: QueueFamilyIndices,
    swapchain_loader: ash::khr::swapchain::Device,
    swapchain: vk::SwapchainKHR,
    swapchain_images: Vec<vk::Image>,
    swapchain_image_views: Vec<vk::ImageView>,
    swapchain_format: vk::Format,
    swapchain_extent: vk::Extent2D,
    render_pass: vk::RenderPass,
    pipeline_layout: vk::PipelineLayout,
    graphics_pipeline: vk::Pipeline,
    framebuffers: Vec<vk::Framebuffer>,
    command_pool: vk::CommandPool,
    command_buffers: Vec<vk::CommandBuffer>,
    image_available: vk::Semaphore,
    render_finished: Vec<vk::Semaphore>,
    in_flight: vk::Fence,
    clear_color: vk::ClearValue,
    framebuffer_resized: bool,
    occluded: bool,
}

impl VulkanRenderer {
    fn new(
        window: Window,
        renderer_config: RendererConfig,
        window_config: VulkanWindowConfig,
    ) -> RenderResult<Self> {
        info!("Loading Vulkan loader");
        let entry = unsafe { Entry::load()? };
        log_instance_layers_and_extensions(&entry)?;

        let validation_enabled = renderer_config.features.enable_validation_layers
            && validation_layer_available(&entry)?;
        if renderer_config.features.enable_validation_layers && !validation_enabled {
            warn!("{VALIDATION_LAYER:?} is not available; continuing without validation layers");
        }
        info!("Validation layers enabled: {validation_enabled}");

        let display_handle = window.display_handle()?.as_raw();
        let required_extensions = ash_window::enumerate_required_extensions(display_handle)?;
        let mut extension_names = required_extensions.to_vec();
        if validation_enabled {
            extension_names.push(ash::ext::debug_utils::NAME.as_ptr());
        }
        log_enabled_extensions("Instance extensions", &extension_names);

        let layer_names = if validation_enabled {
            vec![VALIDATION_LAYER.as_ptr()]
        } else {
            Vec::new()
        };
        log_enabled_extensions("Instance layers", &layer_names);

        let app_name = CString::new("FinalEngine").expect("static app name is valid");
        let engine_name = CString::new("FinalEngine").expect("static engine name is valid");
        let app_info = vk::ApplicationInfo::default()
            .application_name(&app_name)
            .application_version(vk::make_api_version(0, 0, 1, 0))
            .engine_name(&engine_name)
            .engine_version(vk::make_api_version(0, 0, 1, 0))
            .api_version(vk::API_VERSION_1_3);

        let mut debug_create_info = debug_messenger_create_info();
        let mut instance_create_info = vk::InstanceCreateInfo::default()
            .application_info(&app_info)
            .enabled_extension_names(&extension_names)
            .enabled_layer_names(&layer_names);

        if validation_enabled {
            instance_create_info = instance_create_info.push_next(&mut debug_create_info);
        }

        info!("Creating Vulkan instance");
        let instance = unsafe { entry.create_instance(&instance_create_info, None)? };
        let debug_utils =
            validation_enabled.then(|| ash::ext::debug_utils::Instance::new(&entry, &instance));
        let debug_messenger = if let Some(debug_utils) = debug_utils.as_ref() {
            info!("Creating Vulkan debug messenger");
            Some(unsafe { debug_utils.create_debug_utils_messenger(&debug_create_info, None)? })
        } else {
            None
        };

        info!("Creating window surface");
        let surface = unsafe {
            ash_window::create_surface(
                &entry,
                &instance,
                window.display_handle()?.as_raw(),
                window.window_handle()?.as_raw(),
                None,
            )?
        };
        let surface_loader = ash::khr::surface::Instance::new(&entry, &instance);

        let physical_device = pick_physical_device(
            &instance,
            &surface_loader,
            surface,
            &renderer_config.features,
        )?;
        let queue_family_indices =
            find_queue_families(&instance, &surface_loader, surface, physical_device)?.ok_or_else(
                || RenderError::Message("selected device lost queue support".to_string()),
            )?;

        info!(
            "Selected queue families: graphics={}, present={}",
            queue_family_indices.graphics, queue_family_indices.present
        );

        let unique_queue_families = queue_family_indices.unique();
        let queue_priority = [1.0_f32];
        let queue_create_infos = unique_queue_families
            .iter()
            .map(|family| {
                vk::DeviceQueueCreateInfo::default()
                    .queue_family_index(*family)
                    .queue_priorities(&queue_priority)
            })
            .collect::<Vec<_>>();

        let device_extensions = [ash::khr::swapchain::NAME.as_ptr()];
        log_enabled_extensions("Device extensions", &device_extensions);
        let device_features = vk::PhysicalDeviceFeatures::default();
        let device_create_info = vk::DeviceCreateInfo::default()
            .queue_create_infos(&queue_create_infos)
            .enabled_extension_names(&device_extensions)
            .enabled_features(&device_features);

        info!("Creating Vulkan logical device");
        let device = unsafe { instance.create_device(physical_device, &device_create_info, None)? };
        let graphics_queue = unsafe { device.get_device_queue(queue_family_indices.graphics, 0) };
        let present_queue = unsafe { device.get_device_queue(queue_family_indices.present, 0) };
        info!("Retrieved graphics and present queues");

        let swapchain_loader = ash::khr::swapchain::Device::new(&instance, &device);
        let swapchain_support = query_swapchain_support(physical_device, &surface_loader, surface)?;
        log_swapchain_support(&swapchain_support);

        let window_size = window.inner_size();
        let swapchain_bundle = create_swapchain_bundle(
            &device,
            &swapchain_loader,
            surface,
            &swapchain_support,
            queue_family_indices,
            window_size,
        )?;

        let command_pool_create_info = vk::CommandPoolCreateInfo::default()
            .queue_family_index(queue_family_indices.graphics)
            .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER);
        info!("Creating command pool");
        let command_pool = unsafe { device.create_command_pool(&command_pool_create_info, None)? };

        let command_buffers =
            allocate_command_buffers(&device, command_pool, swapchain_bundle.framebuffers.len())?;
        let semaphore_create_info = vk::SemaphoreCreateInfo::default();
        let fence_create_info =
            vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED);
        info!("Creating frame synchronization primitives");
        let image_available = unsafe { device.create_semaphore(&semaphore_create_info, None)? };
        let render_finished =
            create_render_finished_semaphores(&device, swapchain_bundle.framebuffers.len())?;
        let in_flight = unsafe { device.create_fence(&fence_create_info, None)? };

        info!(
            "Renderer initialized: swapchain_images={}, format={:?}, extent={}x{}",
            swapchain_bundle.images.len(),
            swapchain_bundle.format,
            swapchain_bundle.extent.width,
            swapchain_bundle.extent.height
        );

        Ok(Self {
            window,
            _entry: entry,
            instance,
            debug_utils,
            debug_messenger,
            surface_loader,
            surface,
            physical_device,
            device,
            graphics_queue,
            present_queue,
            queue_family_indices,
            swapchain_loader,
            swapchain: swapchain_bundle.swapchain,
            swapchain_images: swapchain_bundle.images,
            swapchain_image_views: swapchain_bundle.image_views,
            swapchain_format: swapchain_bundle.format,
            swapchain_extent: swapchain_bundle.extent,
            render_pass: swapchain_bundle.render_pass,
            pipeline_layout: swapchain_bundle.pipeline_layout,
            graphics_pipeline: swapchain_bundle.graphics_pipeline,
            framebuffers: swapchain_bundle.framebuffers,
            command_pool,
            command_buffers,
            image_available,
            render_finished,
            in_flight,
            clear_color: vk::ClearValue {
                color: vk::ClearColorValue {
                    float32: window_config.clear_color,
                },
            },
            framebuffer_resized: false,
            occluded: false,
        })
    }

    fn resize(&mut self, size: PhysicalSize<u32>) {
        if size.width == 0 || size.height == 0 {
            info!("Window minimized or zero-sized; deferring swapchain recreation");
            return;
        }
        self.framebuffer_resized = true;
    }

    fn mark_swapchain_dirty(&mut self) {
        self.framebuffer_resized = true;
    }

    fn draw_frame(&mut self) -> RenderResult<()> {
        if self.occluded {
            trace!("Skipping draw for occluded window");
            return Ok(());
        }

        let size = self.window.inner_size();
        if size.width == 0 || size.height == 0 {
            trace!("Skipping draw for zero-sized window");
            return Ok(());
        }

        if self.framebuffer_resized {
            info!("Swapchain marked dirty before draw; recreating");
            self.recreate_swapchain()?;
        }

        unsafe {
            self.device
                .wait_for_fences(&[self.in_flight], true, u64::MAX)?;
        }

        let image_index = match unsafe {
            self.swapchain_loader.acquire_next_image(
                self.swapchain,
                u64::MAX,
                self.image_available,
                vk::Fence::null(),
            )
        } {
            Ok((image_index, suboptimal)) => {
                if suboptimal {
                    info!("Swapchain acquire reported suboptimal; will recreate after this frame");
                    self.framebuffer_resized = true;
                }
                image_index
            }
            Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => {
                info!("Swapchain out of date during acquire; recreating");
                self.recreate_swapchain()?;
                return Ok(());
            }
            Err(error) => return Err(error.into()),
        };

        unsafe {
            self.device.reset_fences(&[self.in_flight])?;
            self.device.reset_command_buffer(
                self.command_buffers[image_index as usize],
                vk::CommandBufferResetFlags::empty(),
            )?;
        }

        record_clear_commands(
            &self.device,
            self.command_buffers[image_index as usize],
            self.render_pass,
            self.framebuffers[image_index as usize],
            self.graphics_pipeline,
            self.swapchain_extent,
            self.clear_color,
        )?;

        let wait_semaphores = [self.image_available];
        let wait_stages = [vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT];
        let command_buffers = [self.command_buffers[image_index as usize]];
        let signal_semaphores = [self.render_finished[image_index as usize]];
        let submit_info = vk::SubmitInfo::default()
            .wait_semaphores(&wait_semaphores)
            .wait_dst_stage_mask(&wait_stages)
            .command_buffers(&command_buffers)
            .signal_semaphores(&signal_semaphores);

        unsafe {
            self.device
                .queue_submit(self.graphics_queue, &[submit_info], self.in_flight)?;
        }

        let swapchains = [self.swapchain];
        let image_indices = [image_index];
        let present_info = vk::PresentInfoKHR::default()
            .wait_semaphores(&signal_semaphores)
            .swapchains(&swapchains)
            .image_indices(&image_indices);

        match unsafe {
            self.swapchain_loader
                .queue_present(self.present_queue, &present_info)
        } {
            Ok(suboptimal) => {
                if suboptimal || self.framebuffer_resized {
                    info!("Swapchain present reported suboptimal or resize pending; recreating");
                    self.recreate_swapchain()?;
                }
            }
            Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => {
                info!("Swapchain out of date during present; recreating");
                self.recreate_swapchain()?;
            }
            Err(error) => return Err(error.into()),
        }

        Ok(())
    }

    fn recreate_swapchain(&mut self) -> RenderResult<()> {
        let size = self.window.inner_size();
        if size.width == 0 || size.height == 0 {
            info!("Swapchain recreation skipped for zero-sized window");
            return Ok(());
        }

        info!("Waiting for device idle before swapchain recreation");
        unsafe {
            self.device.device_wait_idle()?;
        }

        self.destroy_swapchain_resources();
        let support =
            query_swapchain_support(self.physical_device, &self.surface_loader, self.surface)?;
        log_swapchain_support(&support);
        let bundle = create_swapchain_bundle(
            &self.device,
            &self.swapchain_loader,
            self.surface,
            &support,
            self.queue_family_indices,
            size,
        )?;
        self.swapchain = bundle.swapchain;
        self.swapchain_images = bundle.images;
        self.swapchain_image_views = bundle.image_views;
        self.swapchain_format = bundle.format;
        self.swapchain_extent = bundle.extent;
        self.render_pass = bundle.render_pass;
        self.pipeline_layout = bundle.pipeline_layout;
        self.graphics_pipeline = bundle.graphics_pipeline;
        self.framebuffers = bundle.framebuffers;
        self.command_buffers =
            allocate_command_buffers(&self.device, self.command_pool, self.framebuffers.len())?;
        self.render_finished =
            create_render_finished_semaphores(&self.device, self.framebuffers.len())?;
        self.framebuffer_resized = false;

        info!(
            "Swapchain recreated: images={}, format={:?}, extent={}x{}",
            self.swapchain_images.len(),
            self.swapchain_format,
            self.swapchain_extent.width,
            self.swapchain_extent.height
        );
        Ok(())
    }

    fn destroy_swapchain_resources(&mut self) {
        info!("Destroying swapchain-dependent resources");
        unsafe {
            if !self.command_buffers.is_empty() {
                self.device
                    .free_command_buffers(self.command_pool, &self.command_buffers);
            }
            self.command_buffers.clear();
            for semaphore in self.render_finished.drain(..) {
                self.device.destroy_semaphore(semaphore, None);
            }

            for framebuffer in self.framebuffers.drain(..) {
                self.device.destroy_framebuffer(framebuffer, None);
            }
            if self.graphics_pipeline != vk::Pipeline::null() {
                self.device.destroy_pipeline(self.graphics_pipeline, None);
                self.graphics_pipeline = vk::Pipeline::null();
            }
            if self.pipeline_layout != vk::PipelineLayout::null() {
                self.device
                    .destroy_pipeline_layout(self.pipeline_layout, None);
                self.pipeline_layout = vk::PipelineLayout::null();
            }
            if self.render_pass != vk::RenderPass::null() {
                self.device.destroy_render_pass(self.render_pass, None);
                self.render_pass = vk::RenderPass::null();
            }
            for image_view in self.swapchain_image_views.drain(..) {
                self.device.destroy_image_view(image_view, None);
            }
            if self.swapchain != vk::SwapchainKHR::null() {
                self.swapchain_loader
                    .destroy_swapchain(self.swapchain, None);
                self.swapchain = vk::SwapchainKHR::null();
            }
        }
    }
}

impl Drop for VulkanRenderer {
    fn drop(&mut self) {
        info!("Dropping Vulkan renderer and destroying GPU resources");
        unsafe {
            if let Err(error) = self.device.device_wait_idle() {
                warn!("device_wait_idle failed during drop: {error:?}");
            }
            self.destroy_swapchain_resources();
            self.device.destroy_fence(self.in_flight, None);
            self.device.destroy_semaphore(self.image_available, None);
            self.device.destroy_command_pool(self.command_pool, None);
            self.device.destroy_device(None);
            self.surface_loader.destroy_surface(self.surface, None);
            if let (Some(debug_utils), Some(debug_messenger)) =
                (self.debug_utils.as_ref(), self.debug_messenger)
            {
                debug_utils.destroy_debug_utils_messenger(debug_messenger, None);
            }
            self.instance.destroy_instance(None);
        }
        info!("Vulkan resources destroyed");
    }
}

#[derive(Debug, Clone, Copy)]
struct QueueFamilyIndices {
    graphics: u32,
    present: u32,
}

impl QueueFamilyIndices {
    fn unique(self) -> Vec<u32> {
        BTreeSet::from([self.graphics, self.present])
            .into_iter()
            .collect()
    }
}

#[derive(Debug)]
struct SwapchainSupport {
    capabilities: vk::SurfaceCapabilitiesKHR,
    formats: Vec<vk::SurfaceFormatKHR>,
    present_modes: Vec<vk::PresentModeKHR>,
}

struct SwapchainBundle {
    swapchain: vk::SwapchainKHR,
    images: Vec<vk::Image>,
    image_views: Vec<vk::ImageView>,
    format: vk::Format,
    extent: vk::Extent2D,
    render_pass: vk::RenderPass,
    pipeline_layout: vk::PipelineLayout,
    graphics_pipeline: vk::Pipeline,
    framebuffers: Vec<vk::Framebuffer>,
}

fn log_instance_layers_and_extensions(entry: &Entry) -> RenderResult<()> {
    let layers = unsafe { entry.enumerate_instance_layer_properties()? };
    info!("Available Vulkan instance layers: {}", layers.len());
    for layer in layers {
        info!(
            "  layer={} spec={} implementation={} description={}",
            vk_string(&layer.layer_name),
            layer.spec_version,
            layer.implementation_version,
            vk_string(&layer.description)
        );
    }

    let extensions = unsafe { entry.enumerate_instance_extension_properties(None)? };
    info!("Available Vulkan instance extensions: {}", extensions.len());
    for extension in extensions {
        info!(
            "  extension={} spec={}",
            vk_string(&extension.extension_name),
            extension.spec_version
        );
    }
    Ok(())
}

fn validation_layer_available(entry: &Entry) -> RenderResult<bool> {
    let layers = unsafe { entry.enumerate_instance_layer_properties()? };
    Ok(layers
        .iter()
        .any(|layer| unsafe { CStr::from_ptr(layer.layer_name.as_ptr()) } == VALIDATION_LAYER))
}

fn log_enabled_extensions(label: &str, names: &[*const i8]) {
    info!("{label}: {}", names.len());
    for name in names {
        let name = unsafe { CStr::from_ptr(*name) };
        info!("  {}", name.to_string_lossy());
    }
}

fn pick_physical_device(
    instance: &Instance,
    surface_loader: &ash::khr::surface::Instance,
    surface: vk::SurfaceKHR,
    policy: &VulkanFeaturePolicy,
) -> RenderResult<vk::PhysicalDevice> {
    let devices = unsafe { instance.enumerate_physical_devices()? };
    info!("Enumerating Vulkan physical devices: {}", devices.len());
    let mut best: Option<(i32, vk::PhysicalDevice)> = None;

    for device in devices {
        let properties = unsafe { instance.get_physical_device_properties(device) };
        let features = unsafe { instance.get_physical_device_features(device) };
        let api = properties.api_version;
        let name = vk_string(&properties.device_name);
        let device_type = properties.device_type;
        info!("Physical device: {name}");
        info!("  type={device_type:?}");
        info!(
            "  api={}.{}.{}",
            vk::api_version_major(api),
            vk::api_version_minor(api),
            vk::api_version_patch(api)
        );
        info!(
            "  vendor_id=0x{:x} device_id=0x{:x}",
            properties.vendor_id, properties.device_id
        );
        info!("  geometry_shader={}", features.geometry_shader == vk::TRUE);
        info!(
            "  sampler_anisotropy={}",
            features.sampler_anisotropy == vk::TRUE
        );

        let extensions = unsafe { instance.enumerate_device_extension_properties(device)? };
        info!("  device extensions={}", extensions.len());
        for extension in &extensions {
            info!(
                "    {} spec={}",
                vk_string(&extension.extension_name),
                extension.spec_version
            );
        }

        let has_swapchain = extensions.iter().any(|extension| {
            let extension_name = unsafe { CStr::from_ptr(extension.extension_name.as_ptr()) };
            extension_name == ash::khr::swapchain::NAME
        });
        let queue_families = find_queue_families(instance, surface_loader, surface, device)?;
        let swapchain_support = query_swapchain_support(device, surface_loader, surface)?;
        let supports_vulkan_1_3 = vk::api_version_major(api) > 1 || vk::api_version_minor(api) >= 3;
        let usable = has_swapchain
            && queue_families.is_some()
            && !swapchain_support.formats.is_empty()
            && !swapchain_support.present_modes.is_empty()
            && (!policy.require_vulkan_1_3 || supports_vulkan_1_3);

        info!("  has_swapchain_extension={has_swapchain}");
        info!("  has_required_queues={}", queue_families.is_some());
        info!("  surface_formats={}", swapchain_support.formats.len());
        info!("  present_modes={}", swapchain_support.present_modes.len());
        info!("  supports_vulkan_1_3={supports_vulkan_1_3}");
        info!("  usable={usable}");

        if !usable {
            continue;
        }

        let score = match device_type {
            vk::PhysicalDeviceType::DISCRETE_GPU => 10_000,
            vk::PhysicalDeviceType::INTEGRATED_GPU => 5_000,
            vk::PhysicalDeviceType::VIRTUAL_GPU => 2_500,
            vk::PhysicalDeviceType::CPU => 500,
            _ => 100,
        } + properties.limits.max_image_dimension2_d as i32;

        info!("  selection_score={score}");
        if best.is_none_or(|(best_score, _)| score > best_score) {
            best = Some((score, device));
        }
    }

    best.map(|(_, device)| device)
        .ok_or_else(|| RenderError::Message("no suitable Vulkan physical device found".to_string()))
}

fn find_queue_families(
    instance: &Instance,
    surface_loader: &ash::khr::surface::Instance,
    surface: vk::SurfaceKHR,
    device: vk::PhysicalDevice,
) -> RenderResult<Option<QueueFamilyIndices>> {
    let queue_families = unsafe { instance.get_physical_device_queue_family_properties(device) };
    let mut graphics = None;
    let mut present = None;

    for (index, family) in queue_families.iter().enumerate() {
        let index = index as u32;
        let supports_graphics = family.queue_flags.contains(vk::QueueFlags::GRAPHICS);
        let supports_present =
            unsafe { surface_loader.get_physical_device_surface_support(device, index, surface)? };
        info!(
            "  queue_family[{index}]: count={} flags={:?} graphics={} present={}",
            family.queue_count, family.queue_flags, supports_graphics, supports_present
        );

        if supports_graphics && graphics.is_none() {
            graphics = Some(index);
        }
        if supports_present && present.is_none() {
            present = Some(index);
        }
    }

    Ok(match (graphics, present) {
        (Some(graphics), Some(present)) => Some(QueueFamilyIndices { graphics, present }),
        _ => None,
    })
}

fn query_swapchain_support(
    device: vk::PhysicalDevice,
    surface_loader: &ash::khr::surface::Instance,
    surface: vk::SurfaceKHR,
) -> RenderResult<SwapchainSupport> {
    Ok(SwapchainSupport {
        capabilities: unsafe {
            surface_loader.get_physical_device_surface_capabilities(device, surface)?
        },
        formats: unsafe { surface_loader.get_physical_device_surface_formats(device, surface)? },
        present_modes: unsafe {
            surface_loader.get_physical_device_surface_present_modes(device, surface)?
        },
    })
}

fn log_swapchain_support(support: &SwapchainSupport) {
    let caps = support.capabilities;
    info!("Swapchain capabilities:");
    info!(
        "  min_images={} max_images={} current_extent={}x{} min_extent={}x{} max_extent={}x{} current_transform={:?}",
        caps.min_image_count,
        caps.max_image_count,
        caps.current_extent.width,
        caps.current_extent.height,
        caps.min_image_extent.width,
        caps.min_image_extent.height,
        caps.max_image_extent.width,
        caps.max_image_extent.height,
        caps.current_transform
    );
    info!("  formats={}", support.formats.len());
    for format in &support.formats {
        info!(
            "    format={:?} color_space={:?}",
            format.format, format.color_space
        );
    }
    info!("  present_modes={}", support.present_modes.len());
    for present_mode in &support.present_modes {
        info!("    {present_mode:?}");
    }
}

fn create_swapchain_bundle(
    device: &Device,
    swapchain_loader: &ash::khr::swapchain::Device,
    surface: vk::SurfaceKHR,
    support: &SwapchainSupport,
    queue_family_indices: QueueFamilyIndices,
    window_size: PhysicalSize<u32>,
) -> RenderResult<SwapchainBundle> {
    let surface_format = choose_surface_format(&support.formats);
    let present_mode = choose_present_mode(&support.present_modes);
    let extent = choose_swap_extent(&support.capabilities, window_size);
    let mut image_count = support.capabilities.min_image_count + 1;
    if support.capabilities.max_image_count > 0 {
        image_count = image_count.min(support.capabilities.max_image_count);
    }

    let unique_families = queue_family_indices.unique();
    let sharing_mode = if unique_families.len() > 1 {
        vk::SharingMode::CONCURRENT
    } else {
        vk::SharingMode::EXCLUSIVE
    };

    info!(
        "Creating swapchain: images={} format={:?} color_space={:?} present_mode={:?} extent={}x{} sharing={:?}",
        image_count,
        surface_format.format,
        surface_format.color_space,
        present_mode,
        extent.width,
        extent.height,
        sharing_mode
    );

    let swapchain_create_info = vk::SwapchainCreateInfoKHR::default()
        .surface(surface)
        .min_image_count(image_count)
        .image_format(surface_format.format)
        .image_color_space(surface_format.color_space)
        .image_extent(extent)
        .image_array_layers(1)
        .image_usage(vk::ImageUsageFlags::COLOR_ATTACHMENT)
        .image_sharing_mode(sharing_mode)
        .queue_family_indices(&unique_families)
        .pre_transform(support.capabilities.current_transform)
        .composite_alpha(vk::CompositeAlphaFlagsKHR::OPAQUE)
        .present_mode(present_mode)
        .clipped(true);

    let swapchain = unsafe { swapchain_loader.create_swapchain(&swapchain_create_info, None)? };
    let images = unsafe { swapchain_loader.get_swapchain_images(swapchain)? };
    info!("Swapchain returned {} images", images.len());

    let image_views = images
        .iter()
        .map(|image| create_image_view(device, *image, surface_format.format))
        .collect::<RenderResult<Vec<_>>>()?;
    let render_pass = create_render_pass(device, surface_format.format)?;
    let (pipeline_layout, graphics_pipeline) =
        create_triangle_pipeline(device, render_pass, extent)?;
    let framebuffers = image_views
        .iter()
        .map(|image_view| create_framebuffer(device, render_pass, *image_view, extent))
        .collect::<RenderResult<Vec<_>>>()?;

    Ok(SwapchainBundle {
        swapchain,
        images,
        image_views,
        format: surface_format.format,
        extent,
        render_pass,
        pipeline_layout,
        graphics_pipeline,
        framebuffers,
    })
}

fn choose_surface_format(formats: &[vk::SurfaceFormatKHR]) -> vk::SurfaceFormatKHR {
    let selected = formats
        .iter()
        .copied()
        .find(|format| {
            format.format == vk::Format::B8G8R8A8_SRGB
                && format.color_space == vk::ColorSpaceKHR::SRGB_NONLINEAR
        })
        .unwrap_or_else(|| formats[0]);
    info!(
        "Selected surface format: {:?} / {:?}",
        selected.format, selected.color_space
    );
    selected
}

fn choose_present_mode(present_modes: &[vk::PresentModeKHR]) -> vk::PresentModeKHR {
    let selected = present_modes
        .iter()
        .copied()
        .find(|mode| *mode == vk::PresentModeKHR::MAILBOX)
        .unwrap_or(vk::PresentModeKHR::FIFO);
    info!("Selected present mode: {selected:?}");
    selected
}

fn choose_swap_extent(
    capabilities: &vk::SurfaceCapabilitiesKHR,
    window_size: PhysicalSize<u32>,
) -> vk::Extent2D {
    if capabilities.current_extent.width != u32::MAX {
        return capabilities.current_extent;
    }

    vk::Extent2D {
        width: window_size.width.clamp(
            capabilities.min_image_extent.width,
            capabilities.max_image_extent.width,
        ),
        height: window_size.height.clamp(
            capabilities.min_image_extent.height,
            capabilities.max_image_extent.height,
        ),
    }
}

fn create_image_view(
    device: &Device,
    image: vk::Image,
    format: vk::Format,
) -> RenderResult<vk::ImageView> {
    let subresource_range = vk::ImageSubresourceRange::default()
        .aspect_mask(vk::ImageAspectFlags::COLOR)
        .base_mip_level(0)
        .level_count(1)
        .base_array_layer(0)
        .layer_count(1);
    let create_info = vk::ImageViewCreateInfo::default()
        .image(image)
        .view_type(vk::ImageViewType::TYPE_2D)
        .format(format)
        .components(vk::ComponentMapping::default())
        .subresource_range(subresource_range);
    Ok(unsafe { device.create_image_view(&create_info, None)? })
}

fn create_render_pass(device: &Device, format: vk::Format) -> RenderResult<vk::RenderPass> {
    let color_attachment = vk::AttachmentDescription::default()
        .format(format)
        .samples(vk::SampleCountFlags::TYPE_1)
        .load_op(vk::AttachmentLoadOp::CLEAR)
        .store_op(vk::AttachmentStoreOp::STORE)
        .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
        .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
        .initial_layout(vk::ImageLayout::UNDEFINED)
        .final_layout(vk::ImageLayout::PRESENT_SRC_KHR);
    let color_attachment_ref = vk::AttachmentReference::default()
        .attachment(0)
        .layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL);
    let color_attachments = [color_attachment_ref];
    let subpass = vk::SubpassDescription::default()
        .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
        .color_attachments(&color_attachments);
    let dependency = vk::SubpassDependency::default()
        .src_subpass(vk::SUBPASS_EXTERNAL)
        .dst_subpass(0)
        .src_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
        .src_access_mask(vk::AccessFlags::empty())
        .dst_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
        .dst_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE);
    let attachments = [color_attachment];
    let subpasses = [subpass];
    let dependencies = [dependency];
    let render_pass_info = vk::RenderPassCreateInfo::default()
        .attachments(&attachments)
        .subpasses(&subpasses)
        .dependencies(&dependencies);
    info!("Creating render pass for format {format:?}");
    Ok(unsafe { device.create_render_pass(&render_pass_info, None)? })
}

fn create_triangle_pipeline(
    device: &Device,
    render_pass: vk::RenderPass,
    extent: vk::Extent2D,
) -> RenderResult<(vk::PipelineLayout, vk::Pipeline)> {
    info!(
        "Creating triangle graphics pipeline for extent {}x{}",
        extent.width, extent.height
    );
    info!(
        "Embedded triangle shaders: vertex={} bytes, fragment={} bytes",
        TRIANGLE_VERTEX_SHADER.len(),
        TRIANGLE_FRAGMENT_SHADER.len()
    );

    let vertex_shader = create_shader_module(device, TRIANGLE_VERTEX_SHADER, "triangle.vert")?;
    let fragment_shader = create_shader_module(device, TRIANGLE_FRAGMENT_SHADER, "triangle.frag")?;
    let entry_point = c"main";
    let shader_stages = [
        vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::VERTEX)
            .module(vertex_shader)
            .name(entry_point),
        vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::FRAGMENT)
            .module(fragment_shader)
            .name(entry_point),
    ];

    let vertex_input_info = vk::PipelineVertexInputStateCreateInfo::default();
    let input_assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
        .topology(vk::PrimitiveTopology::TRIANGLE_LIST)
        .primitive_restart_enable(false);
    let viewport = vk::Viewport {
        x: 0.0,
        y: 0.0,
        width: extent.width as f32,
        height: extent.height as f32,
        min_depth: 0.0,
        max_depth: 1.0,
    };
    let scissor = vk::Rect2D {
        offset: vk::Offset2D { x: 0, y: 0 },
        extent,
    };
    let viewports = [viewport];
    let scissors = [scissor];
    let viewport_state = vk::PipelineViewportStateCreateInfo::default()
        .viewports(&viewports)
        .scissors(&scissors);
    let rasterizer = vk::PipelineRasterizationStateCreateInfo::default()
        .depth_clamp_enable(false)
        .rasterizer_discard_enable(false)
        .polygon_mode(vk::PolygonMode::FILL)
        .line_width(1.0)
        .cull_mode(vk::CullModeFlags::BACK)
        .front_face(vk::FrontFace::CLOCKWISE)
        .depth_bias_enable(false);
    let multisampling = vk::PipelineMultisampleStateCreateInfo::default()
        .sample_shading_enable(false)
        .rasterization_samples(vk::SampleCountFlags::TYPE_1);
    let color_blend_attachment = vk::PipelineColorBlendAttachmentState::default()
        .color_write_mask(
            vk::ColorComponentFlags::R
                | vk::ColorComponentFlags::G
                | vk::ColorComponentFlags::B
                | vk::ColorComponentFlags::A,
        )
        .blend_enable(false);
    let color_blend_attachments = [color_blend_attachment];
    let color_blending = vk::PipelineColorBlendStateCreateInfo::default()
        .logic_op_enable(false)
        .attachments(&color_blend_attachments);
    let pipeline_layout_info = vk::PipelineLayoutCreateInfo::default();
    let pipeline_layout = unsafe { device.create_pipeline_layout(&pipeline_layout_info, None)? };
    let pipeline_info = vk::GraphicsPipelineCreateInfo::default()
        .stages(&shader_stages)
        .vertex_input_state(&vertex_input_info)
        .input_assembly_state(&input_assembly)
        .viewport_state(&viewport_state)
        .rasterization_state(&rasterizer)
        .multisample_state(&multisampling)
        .color_blend_state(&color_blending)
        .layout(pipeline_layout)
        .render_pass(render_pass)
        .subpass(0);

    let pipeline_result = unsafe {
        device.create_graphics_pipelines(vk::PipelineCache::null(), &[pipeline_info], None)
    };

    unsafe {
        device.destroy_shader_module(fragment_shader, None);
        device.destroy_shader_module(vertex_shader, None);
    }

    match pipeline_result {
        Ok(mut pipelines) => {
            let pipeline = pipelines.pop().ok_or_else(|| {
                RenderError::Message("Vulkan returned no graphics pipeline".to_string())
            })?;
            info!("Triangle graphics pipeline created");
            Ok((pipeline_layout, pipeline))
        }
        Err((pipelines, error)) => {
            unsafe {
                for pipeline in pipelines {
                    device.destroy_pipeline(pipeline, None);
                }
                device.destroy_pipeline_layout(pipeline_layout, None);
            }
            Err(error.into())
        }
    }
}

fn create_shader_module(
    device: &Device,
    bytes: &[u8],
    label: &str,
) -> RenderResult<vk::ShaderModule> {
    info!("Creating shader module {label} from {} bytes", bytes.len());
    let mut cursor = Cursor::new(bytes);
    let code = ash::util::read_spv(&mut cursor)
        .map_err(|error| RenderError::Message(format!("failed to read {label} SPIR-V: {error}")))?;
    let create_info = vk::ShaderModuleCreateInfo::default().code(&code);
    Ok(unsafe { device.create_shader_module(&create_info, None)? })
}

fn create_framebuffer(
    device: &Device,
    render_pass: vk::RenderPass,
    image_view: vk::ImageView,
    extent: vk::Extent2D,
) -> RenderResult<vk::Framebuffer> {
    let attachments = [image_view];
    let framebuffer_info = vk::FramebufferCreateInfo::default()
        .render_pass(render_pass)
        .attachments(&attachments)
        .width(extent.width)
        .height(extent.height)
        .layers(1);
    Ok(unsafe { device.create_framebuffer(&framebuffer_info, None)? })
}

fn allocate_command_buffers(
    device: &Device,
    command_pool: vk::CommandPool,
    count: usize,
) -> RenderResult<Vec<vk::CommandBuffer>> {
    info!("Allocating {count} command buffers");
    let allocate_info = vk::CommandBufferAllocateInfo::default()
        .command_pool(command_pool)
        .level(vk::CommandBufferLevel::PRIMARY)
        .command_buffer_count(count as u32);
    Ok(unsafe { device.allocate_command_buffers(&allocate_info)? })
}

fn create_render_finished_semaphores(
    device: &Device,
    count: usize,
) -> RenderResult<Vec<vk::Semaphore>> {
    info!("Creating {count} render-finished semaphores, one per swapchain image");
    let create_info = vk::SemaphoreCreateInfo::default();
    (0..count)
        .map(|_| Ok(unsafe { device.create_semaphore(&create_info, None)? }))
        .collect()
}

fn record_clear_commands(
    device: &Device,
    command_buffer: vk::CommandBuffer,
    render_pass: vk::RenderPass,
    framebuffer: vk::Framebuffer,
    graphics_pipeline: vk::Pipeline,
    extent: vk::Extent2D,
    clear_color: vk::ClearValue,
) -> RenderResult<()> {
    let begin_info = vk::CommandBufferBeginInfo::default();
    let render_area = vk::Rect2D {
        offset: vk::Offset2D { x: 0, y: 0 },
        extent,
    };
    let clear_values = [clear_color];
    let render_pass_info = vk::RenderPassBeginInfo::default()
        .render_pass(render_pass)
        .framebuffer(framebuffer)
        .render_area(render_area)
        .clear_values(&clear_values);

    unsafe {
        device.begin_command_buffer(command_buffer, &begin_info)?;
        device.cmd_begin_render_pass(
            command_buffer,
            &render_pass_info,
            vk::SubpassContents::INLINE,
        );
        device.cmd_bind_pipeline(
            command_buffer,
            vk::PipelineBindPoint::GRAPHICS,
            graphics_pipeline,
        );
        device.cmd_draw(command_buffer, 3, 1, 0, 0);
        device.cmd_end_render_pass(command_buffer);
        device.end_command_buffer(command_buffer)?;
    }
    Ok(())
}

fn debug_messenger_create_info() -> vk::DebugUtilsMessengerCreateInfoEXT<'static> {
    vk::DebugUtilsMessengerCreateInfoEXT::default()
        .message_severity(
            vk::DebugUtilsMessageSeverityFlagsEXT::VERBOSE
                | vk::DebugUtilsMessageSeverityFlagsEXT::INFO
                | vk::DebugUtilsMessageSeverityFlagsEXT::WARNING
                | vk::DebugUtilsMessageSeverityFlagsEXT::ERROR,
        )
        .message_type(
            vk::DebugUtilsMessageTypeFlagsEXT::GENERAL
                | vk::DebugUtilsMessageTypeFlagsEXT::VALIDATION
                | vk::DebugUtilsMessageTypeFlagsEXT::PERFORMANCE,
        )
        .pfn_user_callback(Some(vulkan_debug_callback))
}

unsafe extern "system" fn vulkan_debug_callback(
    severity: vk::DebugUtilsMessageSeverityFlagsEXT,
    message_type: vk::DebugUtilsMessageTypeFlagsEXT,
    callback_data: *const vk::DebugUtilsMessengerCallbackDataEXT<'_>,
    _user_data: *mut c_void,
) -> vk::Bool32 {
    let message = if callback_data.is_null() {
        "<null callback data>".into()
    } else {
        let raw_message = unsafe { (*callback_data).p_message };
        if raw_message.is_null() {
            "<null message>".into()
        } else {
            unsafe { CStr::from_ptr(raw_message) }.to_string_lossy()
        }
    };

    if severity.contains(vk::DebugUtilsMessageSeverityFlagsEXT::ERROR) {
        error!("Vulkan validation [{message_type:?}]: {message}");
    } else if severity.contains(vk::DebugUtilsMessageSeverityFlagsEXT::WARNING) {
        warn!("Vulkan validation [{message_type:?}]: {message}");
    } else if severity.contains(vk::DebugUtilsMessageSeverityFlagsEXT::INFO) {
        info!("Vulkan validation [{message_type:?}]: {message}");
    } else {
        debug!("Vulkan validation [{message_type:?}]: {message}");
    }

    vk::FALSE
}

fn vk_string(raw: &[i8]) -> String {
    unsafe { CStr::from_ptr(raw.as_ptr()) }
        .to_string_lossy()
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renderer_uses_raster_fallback_without_rt() {
        let config = RendererConfig::vulkan_high_end();

        assert_eq!(config.choose_path(false), RendererPath::Raster);
    }

    #[test]
    fn renderer_prefers_rt_when_available() {
        let config = RendererConfig::vulkan_high_end();

        assert_eq!(config.choose_path(true), RendererPath::RayTracing);
    }

    #[test]
    fn swap_extent_clamps_to_surface_limits() {
        let capabilities = vk::SurfaceCapabilitiesKHR {
            current_extent: vk::Extent2D {
                width: u32::MAX,
                height: u32::MAX,
            },
            min_image_extent: vk::Extent2D {
                width: 640,
                height: 480,
            },
            max_image_extent: vk::Extent2D {
                width: 1920,
                height: 1080,
            },
            ..Default::default()
        };

        let extent = choose_swap_extent(&capabilities, PhysicalSize::new(4096, 64));

        assert_eq!(extent.width, 1920);
        assert_eq!(extent.height, 480);
    }
}
