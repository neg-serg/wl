//! Wayland client connection module for the Vulkan wallpaper daemon.
//!
//! Connects to the Wayland display, binds required protocol globals (compositor,
//! layer-shell, fractional-scale), tracks outputs with resolution and scale data,
//! and provides raw `wl_display` / `wl_surface` pointers for Vulkan surface creation.

use std::ffi::c_void;

use wayland_client::protocol::{wl_compositor, wl_output, wl_registry, wl_surface};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle};
use wayland_protocols::wp::fractional_scale::v1::client::{
    wp_fractional_scale_manager_v1, wp_fractional_scale_v1,
};
use wayland_protocols::wp::viewporter::client::{wp_viewport, wp_viewporter};
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

#[derive(Debug)]
#[allow(dead_code)]
pub enum WaylandError {
    /// Failed to connect to the Wayland display.
    Connect(wayland_client::ConnectError),
    /// A Wayland protocol or dispatch error.
    Dispatch(wayland_client::DispatchError),
    /// A required global was not advertised by the compositor.
    MissingGlobal(&'static str),
    /// Requested output index is out of range.
    InvalidOutputIndex(usize),
    /// The layer surface has not been configured yet.
    NotConfigured,
    /// Generic backend error.
    Backend(wayland_client::backend::WaylandError),
}

impl std::fmt::Display for WaylandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Connect(e) => write!(f, "wayland connect: {e}"),
            Self::Dispatch(e) => write!(f, "wayland dispatch: {e}"),
            Self::MissingGlobal(name) => write!(f, "required wayland global not found: {name}"),
            Self::InvalidOutputIndex(i) => write!(f, "output index {i} out of range"),
            Self::NotConfigured => write!(f, "layer surface not yet configured"),
            Self::Backend(e) => write!(f, "wayland backend: {e}"),
        }
    }
}

impl std::error::Error for WaylandError {}

impl From<wayland_client::ConnectError> for WaylandError {
    fn from(e: wayland_client::ConnectError) -> Self {
        Self::Connect(e)
    }
}

impl From<wayland_client::DispatchError> for WaylandError {
    fn from(e: wayland_client::DispatchError) -> Self {
        Self::Dispatch(e)
    }
}

impl From<wayland_client::backend::WaylandError> for WaylandError {
    fn from(e: wayland_client::backend::WaylandError) -> Self {
        Self::Backend(e)
    }
}

// ---------------------------------------------------------------------------
// Per-output state
// ---------------------------------------------------------------------------

pub struct OutputData {
    pub wl_output: wl_output::WlOutput,
    pub name: Option<String>,
    pub width: u32,
    pub height: u32,
    pub refresh_mhz: u32,
    /// Effective scale factor. Updated from `wp_fractional_scale_v1` when available,
    /// otherwise from `wl_output::scale`.
    pub scale_factor: f64,
    pub surface: Option<wl_surface::WlSurface>,
    pub layer_surface: Option<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1>,
    pub configured: bool,
    /// Set when the compositor closes our layer surface. The main loop checks
    /// this flag to tear down Vulkan resources and recreate the surface.
    pub surface_lost: bool,
    pub fractional_scale: Option<wp_fractional_scale_v1::WpFractionalScaleV1>,
    pub viewport: Option<wp_viewport::WpViewport>,
}

impl OutputData {
    fn new(wl_output: wl_output::WlOutput) -> Self {
        Self {
            wl_output,
            name: None,
            width: 0,
            height: 0,
            refresh_mhz: 0,
            scale_factor: 1.0,
            surface: None,
            layer_surface: None,
            configured: false,
            surface_lost: false,
            fractional_scale: None,
            viewport: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Shared dispatch data
// ---------------------------------------------------------------------------

#[allow(dead_code)]
pub struct WaylandData {
    pub compositor: Option<wl_compositor::WlCompositor>,
    pub layer_shell: Option<zwlr_layer_shell_v1::ZwlrLayerShellV1>,
    pub fractional_scale_manager:
        Option<wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1>,
    pub viewporter: Option<wp_viewporter::WpViewporter>,
    pub outputs: Vec<OutputData>,
    pub globals_done: bool,
}

impl WaylandData {
    fn new() -> Self {
        Self {
            compositor: None,
            layer_shell: None,
            fractional_scale_manager: None,
            viewporter: None,
            outputs: Vec::new(),
            globals_done: false,
        }
    }

    /// Find the output that owns a given `wl_output` proxy.
    fn output_mut_by_proxy(&mut self, proxy: &wl_output::WlOutput) -> Option<&mut OutputData> {
        self.outputs.iter_mut().find(|o| o.wl_output == *proxy)
    }

    /// Find the output that owns a given layer-surface proxy.
    fn output_mut_by_layer_surface(
        &mut self,
        ls: &zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
    ) -> Option<&mut OutputData> {
        self.outputs
            .iter_mut()
            .find(|o| o.layer_surface.as_ref() == Some(ls))
    }

    /// Find the output that owns a given fractional-scale proxy.
    fn output_mut_by_fractional_scale(
        &mut self,
        fs: &wp_fractional_scale_v1::WpFractionalScaleV1,
    ) -> Option<&mut OutputData> {
        self.outputs
            .iter_mut()
            .find(|o| o.fractional_scale.as_ref() == Some(fs))
    }
}

// ---------------------------------------------------------------------------
// Top-level state
// ---------------------------------------------------------------------------

#[allow(dead_code)]
pub struct WaylandState {
    pub connection: Connection,
    pub display: wayland_client::protocol::wl_display::WlDisplay,
    pub event_queue: EventQueue<WaylandData>,
    pub data: WaylandData,
}

impl WaylandState {
    /// Connect to the Wayland display, perform an initial roundtrip to discover
    /// globals, and verify that all required globals are present.
    pub fn connect() -> Result<Self, WaylandError> {
        let connection = Connection::connect_to_env()?;
        let display = connection.display();

        let mut event_queue: EventQueue<WaylandData> = connection.new_event_queue();
        let qh = event_queue.handle();

        let mut data = WaylandData::new();

        // Subscribe to the registry to discover globals.
        display.get_registry(&qh, ());

        // First roundtrip: the compositor sends us all global advertisements.
        event_queue.roundtrip(&mut data)?;

        // Second roundtrip: output events (geometry, mode, done) arrive.
        event_queue.roundtrip(&mut data)?;

        tracing::info!(
            outputs = data.outputs.len(),
            compositor = data.compositor.is_some(),
            layer_shell = data.layer_shell.is_some(),
            fractional_scale = data.fractional_scale_manager.is_some(),
            "wayland globals bound",
        );

        if data.compositor.is_none() {
            return Err(WaylandError::MissingGlobal("wl_compositor"));
        }
        if data.layer_shell.is_none() {
            return Err(WaylandError::MissingGlobal("zwlr_layer_shell_v1"));
        }

        Ok(Self {
            connection,
            display,
            event_queue,
            data,
        })
    }

    /// Perform a blocking roundtrip, dispatching all pending events.
    pub fn roundtrip(&mut self) -> Result<(), WaylandError> {
        self.event_queue.roundtrip(&mut self.data)?;
        Ok(())
    }

    /// Dispatch pending events without blocking.
    pub fn dispatch_pending(&mut self) -> Result<usize, WaylandError> {
        let n = self.event_queue.dispatch_pending(&mut self.data)?;
        Ok(n)
    }

    /// Flush the outgoing request buffer.
    pub fn flush(&self) -> Result<(), WaylandError> {
        self.connection.flush()?;
        Ok(())
    }

    /// Create a wlr-layer-shell surface on the background layer for the output
    /// at `output_index`. The surface is anchored to all four edges, has
    /// exclusive zone -1 (below everything), and delegates sizing to the
    /// compositor (0x0).
    ///
    /// If a fractional-scale manager is available, a `wp_fractional_scale_v1`
    /// object is also created for the surface.
    pub fn create_layer_surface(&mut self, output_index: usize) -> Result<(), WaylandError> {
        let qh = self.event_queue.handle();

        let compositor = self
            .data
            .compositor
            .as_ref()
            .ok_or(WaylandError::MissingGlobal("wl_compositor"))?;
        let layer_shell = self
            .data
            .layer_shell
            .as_ref()
            .ok_or(WaylandError::MissingGlobal("zwlr_layer_shell_v1"))?;

        let output = self
            .data
            .outputs
            .get(output_index)
            .ok_or(WaylandError::InvalidOutputIndex(output_index))?;

        let surface = compositor.create_surface(&qh, ());
        let layer_surface = layer_shell.get_layer_surface(
            &surface,
            Some(&output.wl_output),
            zwlr_layer_shell_v1::Layer::Background,
            "wl".to_string(),
            &qh,
            (),
        );

        // Anchor to all four edges so the surface spans the entire output.
        layer_surface.set_anchor(
            zwlr_layer_surface_v1::Anchor::Top
                | zwlr_layer_surface_v1::Anchor::Bottom
                | zwlr_layer_surface_v1::Anchor::Left
                | zwlr_layer_surface_v1::Anchor::Right,
        );
        // Exclusive zone -1: do not reserve space; render below all other layers.
        layer_surface.set_exclusive_zone(-1);
        // Size 0x0: let the compositor decide based on the output dimensions.
        layer_surface.set_size(0, 0);

        // Commit to trigger the initial configure from the compositor.
        surface.commit();

        // Optionally attach fractional-scale tracking.
        let fractional_scale = self
            .data
            .fractional_scale_manager
            .as_ref()
            .map(|mgr| mgr.get_fractional_scale(&surface, &qh, ()));

        // Create viewport for HiDPI: buffer at physical resolution, displayed at logical size.
        let viewport = self
            .data
            .viewporter
            .as_ref()
            .map(|vp| vp.get_viewport(&surface, &qh, ()));

        let output = &mut self.data.outputs[output_index];
        output.surface = Some(surface);
        output.layer_surface = Some(layer_surface);
        output.fractional_scale = fractional_scale;
        output.viewport = viewport;
        output.configured = false;
        output.surface_lost = false;

        tracing::debug!(
            output_index,
            name = ?output.name,
            "layer surface created for output",
        );

        Ok(())
    }

    /// Create layer surfaces for every discovered output that does not already
    /// have one.
    pub fn create_all_layer_surfaces(&mut self) -> Result<(), WaylandError> {
        let count = self.data.outputs.len();
        for i in 0..count {
            if self.data.outputs[i].surface.is_none() {
                self.create_layer_surface(i)?;
            }
        }
        Ok(())
    }

    /// Return the raw `wl_display` pointer suitable for Vulkan surface creation
    /// (e.g. `VkWaylandSurfaceCreateInfoKHR::display`).
    ///
    /// # Safety
    /// The returned pointer is valid for the lifetime of this `WaylandState`.
    pub fn get_display_ptr(&self) -> *mut c_void {
        // The wl_display object always has protocol ID 1 in the Wayland protocol.
        // We use the connection's display_id() to get the ObjectId, then use
        // wayland-sys to get the raw wl_display* through wl_display_connect.
        // Actually, the simplest approach: get the file descriptor and use
        // wl_display_connect_to_fd.
        //
        // Better approach: The display ObjectId's protocol_id is always 1.
        // Since we're using the system backend (libwayland-client), the
        // internal wl_display* is the actual C object. We extract it through
        // the connection's poll_fd and wl_display_connect_to_fd — but that
        // creates a NEW display.
        //
        // The correct approach for wayland-client 0.31 with system backend:
        // Use transmute to access the inner sys::client::Backend which has
        // display_ptr(). The public Backend wraps InnerBackend which IS the
        // sys Backend.
        //
        // SAFETY: With the `system` feature, wayland_client::backend::Backend
        // is a newtype around wayland_backend::sys::client::Backend. We
        // transmute to access display_ptr(). This is sound because the layout
        // is identical (single-field struct).
        let backend = self.connection.backend();
        let sys_backend: &wayland_backend::sys::client::Backend =
            // SAFETY: Backend is #[repr(transparent)] over InnerBackend which
            // is sys::client::Backend when client_system feature is enabled.
            unsafe { std::mem::transmute(&backend) };
        sys_backend.display_ptr() as *mut c_void
    }

    /// Return the raw `wl_surface` pointer for the output at `output_index`.
    /// Returns `None` if the output has no surface yet.
    ///
    /// The pointer is suitable for `VkWaylandSurfaceCreateInfoKHR::surface`.
    pub fn get_surface_ptr(&self, output_index: usize) -> Option<*mut c_void> {
        let output = self.data.outputs.get(output_index)?;
        let surface = output.surface.as_ref()?;
        let id = surface.id();
        // SAFETY: With the `system` feature, wayland_client::backend::ObjectId
        // is a newtype around wayland_backend::sys::client::ObjectId. We transmute
        // to access as_ptr() which returns the raw wl_proxy* (== wl_surface*).
        let sys_id: &wayland_backend::sys::client::ObjectId = unsafe { std::mem::transmute(&id) };
        let ptr = sys_id.as_ptr();
        Some(ptr as *mut c_void)
    }

    /// Read-only access to the output list.
    pub fn outputs(&self) -> &[OutputData] {
        &self.data.outputs
    }

    /// Return indices of outputs whose layer surface was closed by the compositor.
    pub fn lost_surface_indices(&self) -> Vec<usize> {
        self.data
            .outputs
            .iter()
            .enumerate()
            .filter(|(_, o)| o.surface_lost)
            .map(|(i, _)| i)
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Dispatch: wl_registry
// ---------------------------------------------------------------------------

impl Dispatch<wl_registry::WlRegistry, ()> for WaylandData {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _data: &(),
        _conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } => match interface.as_str() {
                "wl_compositor" => {
                    let compositor = registry.bind::<wl_compositor::WlCompositor, _, _>(
                        name,
                        version.min(6),
                        qh,
                        (),
                    );
                    tracing::debug!(version, "bound wl_compositor");
                    state.compositor = Some(compositor);
                }
                "zwlr_layer_shell_v1" => {
                    let ls = registry.bind::<zwlr_layer_shell_v1::ZwlrLayerShellV1, _, _>(
                        name,
                        version.min(4),
                        qh,
                        (),
                    );
                    tracing::debug!(version, "bound zwlr_layer_shell_v1");
                    state.layer_shell = Some(ls);
                }
                "wp_fractional_scale_manager_v1" => {
                    let mgr = registry
                        .bind::<wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1, _, _>(
                            name,
                            version.min(1),
                            qh,
                            (),
                        );
                    tracing::debug!(version, "bound wp_fractional_scale_manager_v1");
                    state.fractional_scale_manager = Some(mgr);
                }
                "wp_viewporter" => {
                    let vp = registry.bind::<wp_viewporter::WpViewporter, _, _>(
                        name,
                        version.min(1),
                        qh,
                        (),
                    );
                    tracing::debug!(version, "bound wp_viewporter");
                    state.viewporter = Some(vp);
                }
                "wl_output" => {
                    let output =
                        registry.bind::<wl_output::WlOutput, _, _>(name, version.min(4), qh, ());
                    tracing::debug!(name, version, "discovered wl_output");
                    state.outputs.push(OutputData::new(output));
                }
                _ => {}
            },
            wl_registry::Event::GlobalRemove { name: _ } => {
                // Output removal is handled opportunistically: callers should
                // poll `outputs()` after each roundtrip. A more sophisticated
                // implementation would match on the global name stored during
                // bind, but for now we rely on wl_output::Event::Release or
                // destroy callbacks if available.
                //
                // For wl_output specifically, the compositor sends
                // `wl_output::Event::Name` / mode events, and on removal the
                // proxy becomes inert. We leave cleanup to the caller.
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Dispatch: wl_compositor
// ---------------------------------------------------------------------------

impl Dispatch<wl_compositor::WlCompositor, ()> for WaylandData {
    fn event(
        _state: &mut Self,
        _proxy: &wl_compositor::WlCompositor,
        _event: wl_compositor::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        // wl_compositor has no events.
    }
}

// ---------------------------------------------------------------------------
// Dispatch: wl_surface
// ---------------------------------------------------------------------------

impl Dispatch<wl_surface::WlSurface, ()> for WaylandData {
    fn event(
        _state: &mut Self,
        _proxy: &wl_surface::WlSurface,
        _event: wl_surface::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        // Surface enter/leave events are informational; we ignore them for now.
    }
}

// ---------------------------------------------------------------------------
// Dispatch: wl_output
// ---------------------------------------------------------------------------

impl Dispatch<wl_output::WlOutput, ()> for WaylandData {
    fn event(
        state: &mut Self,
        proxy: &wl_output::WlOutput,
        event: wl_output::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        let Some(output) = state.output_mut_by_proxy(proxy) else {
            return;
        };

        match event {
            wl_output::Event::Name { name } => {
                tracing::debug!(name, "output name");
                output.name = Some(name);
            }
            wl_output::Event::Mode {
                flags: _,
                width,
                height,
                refresh,
            } => {
                output.width = width as u32;
                output.height = height as u32;
                output.refresh_mhz = refresh as u32;
                tracing::debug!(
                    width,
                    height,
                    refresh,
                    name = ?output.name,
                    "output mode",
                );
            }
            wl_output::Event::Scale { factor } => {
                // Only apply integer scale if we don't have fractional scale.
                if output.fractional_scale.is_none() {
                    output.scale_factor = f64::from(factor);
                    tracing::debug!(
                        factor,
                        name = ?output.name,
                        "output integer scale",
                    );
                }
            }
            wl_output::Event::Done => {
                tracing::debug!(
                    name = ?output.name,
                    width = output.width,
                    height = output.height,
                    scale = output.scale_factor,
                    refresh_mhz = output.refresh_mhz,
                    "output configuration done",
                );
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Dispatch: zwlr_layer_shell_v1
// ---------------------------------------------------------------------------

impl Dispatch<zwlr_layer_shell_v1::ZwlrLayerShellV1, ()> for WaylandData {
    fn event(
        _state: &mut Self,
        _proxy: &zwlr_layer_shell_v1::ZwlrLayerShellV1,
        _event: zwlr_layer_shell_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        // The layer-shell global itself has no events.
    }
}

// ---------------------------------------------------------------------------
// Dispatch: zwlr_layer_surface_v1
// ---------------------------------------------------------------------------

impl Dispatch<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1, ()> for WaylandData {
    fn event(
        state: &mut Self,
        proxy: &zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_layer_surface_v1::Event::Configure {
                serial,
                width,
                height,
            } => {
                // Acknowledge the configure so the compositor knows we handled it.
                proxy.ack_configure(serial);

                if let Some(output) = state.output_mut_by_layer_surface(proxy) {
                    // Update output dimensions if the compositor gave us non-zero sizes.
                    if width > 0 {
                        output.width = width;
                    }
                    if height > 0 {
                        output.height = height;
                    }
                    output.configured = true;

                    // Set viewport destination to logical size so the compositor
                    // knows the buffer (at physical resolution) maps to this area.
                    if let Some(ref vp) = output.viewport {
                        vp.set_destination(output.width as i32, output.height as i32);
                    }

                    tracing::debug!(
                        width,
                        height,
                        name = ?output.name,
                        "layer surface configured",
                    );

                    // Commit after ack to complete the configure sequence.
                    if let Some(ref surface) = output.surface {
                        surface.commit();
                    }
                }
            }
            zwlr_layer_surface_v1::Event::Closed => {
                tracing::warn!("layer surface closed by compositor — will recreate");
                if let Some(output) = state.output_mut_by_layer_surface(proxy) {
                    // Clean up: destroy the layer surface and surface.
                    if let Some(ls) = output.layer_surface.take() {
                        ls.destroy();
                    }
                    if let Some(fs) = output.fractional_scale.take() {
                        fs.destroy();
                    }
                    if let Some(s) = output.surface.take() {
                        s.destroy();
                    }
                    output.configured = false;
                    // Signal the main loop to recreate Vulkan resources and
                    // a new layer surface for this output.
                    output.surface_lost = true;
                }
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Dispatch: wp_fractional_scale_manager_v1
// ---------------------------------------------------------------------------

impl Dispatch<wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1, ()> for WaylandData {
    fn event(
        _state: &mut Self,
        _proxy: &wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1,
        _event: wp_fractional_scale_manager_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        // The fractional-scale manager global has no events.
    }
}

// ---------------------------------------------------------------------------
// Dispatch: wp_fractional_scale_v1
// ---------------------------------------------------------------------------

impl Dispatch<wp_fractional_scale_v1::WpFractionalScaleV1, ()> for WaylandData {
    fn event(
        state: &mut Self,
        proxy: &wp_fractional_scale_v1::WpFractionalScaleV1,
        event: wp_fractional_scale_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if let wp_fractional_scale_v1::Event::PreferredScale { scale } = event {
            // The protocol sends scale * 120. Convert to a floating-point factor.
            let factor = f64::from(scale) / 120.0;

            if let Some(output) = state.output_mut_by_fractional_scale(proxy) {
                output.scale_factor = factor;
                tracing::debug!(
                    factor,
                    raw = scale,
                    name = ?output.name,
                    "fractional scale update",
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Dispatch: wp_viewporter
// ---------------------------------------------------------------------------

impl Dispatch<wp_viewporter::WpViewporter, ()> for WaylandData {
    fn event(
        _state: &mut Self,
        _proxy: &wp_viewporter::WpViewporter,
        _event: wp_viewporter::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        // The viewporter global has no events.
    }
}

// ---------------------------------------------------------------------------
// Dispatch: wp_viewport
// ---------------------------------------------------------------------------

impl Dispatch<wp_viewport::WpViewport, ()> for WaylandData {
    fn event(
        _state: &mut Self,
        _proxy: &wp_viewport::WpViewport,
        _event: wp_viewport::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        // Viewport has no client-side events.
    }
}

// ---------------------------------------------------------------------------
// Dispatch: wl_display (required by wayland-client)
// ---------------------------------------------------------------------------

impl Dispatch<wayland_client::protocol::wl_display::WlDisplay, ()> for WaylandData {
    fn event(
        _state: &mut Self,
        _proxy: &wayland_client::protocol::wl_display::WlDisplay,
        _event: wayland_client::protocol::wl_display::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        // Display-level events (error, delete_id) are handled internally by
        // wayland-client; no user-level handling needed.
    }
}

// ---------------------------------------------------------------------------
// Dispatch: wl_callback (used internally by roundtrip)
// ---------------------------------------------------------------------------

impl Dispatch<wayland_client::protocol::wl_callback::WlCallback, ()> for WaylandData {
    fn event(
        _state: &mut Self,
        _proxy: &wayland_client::protocol::wl_callback::WlCallback,
        _event: wayland_client::protocol::wl_callback::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        // Callbacks are used for sync/roundtrip; nothing to do here.
    }
}
