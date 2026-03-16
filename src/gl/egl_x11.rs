// EGL-based OpenGL context for X11 windows.
//
// This replaces the GLX backend for environments where GLX child window
// compositing is broken (e.g., XWayland).

use std::ffi::c_void;
use std::os::raw::c_ulong;

use x11::xlib;

use super::{GlConfig, GlError, Profile};

pub use khronos_egl as egl;
use egl::Instance;

#[derive(Debug)]
pub enum CreationFailedError {
    EglInitFailed,
    NoConfig,
    SurfaceCreationFailed,
    ContextCreationFailed,
    MakeCurrentFailed,
}

impl From<egl::Error> for GlError {
    fn from(_: egl::Error) -> Self {
        GlError::CreationFailed(CreationFailedError::EglInitFailed)
    }
}

pub struct GlContext {
    egl: Instance<egl::Dynamic<libloading::Library, egl::EGL1_5>>,
    display: egl::Display,
    surface: egl::Surface,
    context: egl::Context,
}

/// The configuration a window should be created with after calling
/// [GlContext::get_config_and_visual].
pub struct WindowConfig {
    pub depth: u8,
    pub visual: u32,
}

/// Stored config for creating the context after the window exists.
pub struct EglConfig {
    gl_config: GlConfig,
    egl_config: egl::Config,
    egl: Instance<egl::Dynamic<libloading::Library, egl::EGL1_5>>,
    display: egl::Display,
}

impl GlContext {
    /// Create the EGL context for an existing X11 window.
    pub unsafe fn create(
        window: c_ulong, _x_display: *mut xlib::_XDisplay, config: EglConfig,
    ) -> Result<GlContext, GlError> {
        let egl = config.egl;
        let display = config.display;
        let egl_config = config.egl_config;
        let gl_config = config.gl_config;

        // Create window surface.
        let surface = egl
            .create_window_surface(display, egl_config, window as egl::NativeWindowType, None)
            .map_err(|_| GlError::CreationFailed(CreationFailedError::SurfaceCreationFailed))?;

        // Context attributes.
        let (major, minor) = gl_config.version;
        let context_attribs = [
            egl::CONTEXT_MAJOR_VERSION,
            major as egl::Int,
            egl::CONTEXT_MINOR_VERSION,
            minor as egl::Int,
            egl::CONTEXT_OPENGL_PROFILE_MASK,
            match gl_config.profile {
                Profile::Core => egl::CONTEXT_OPENGL_CORE_PROFILE_BIT,
                Profile::Compatibility => egl::CONTEXT_OPENGL_COMPATIBILITY_PROFILE_BIT,
            },
            egl::NONE,
        ];

        egl.bind_api(egl::OPENGL_API)
            .map_err(|_| GlError::CreationFailed(CreationFailedError::ContextCreationFailed))?;

        let context = egl
            .create_context(display, egl_config, None, &context_attribs)
            .map_err(|_| GlError::CreationFailed(CreationFailedError::ContextCreationFailed))?;

        // Make current to verify, then release.
        egl.make_current(display, Some(surface), Some(surface), Some(context))
            .map_err(|_| GlError::CreationFailed(CreationFailedError::MakeCurrentFailed))?;

        if !gl_config.vsync {
            let _ = egl.swap_interval(display, 0);
        }

        egl.make_current(display, None, None, None)
            .map_err(|_| GlError::CreationFailed(CreationFailedError::MakeCurrentFailed))?;

        Ok(GlContext { egl, display, surface, context })
    }

    /// Find a matching EGL config and X11 visual.
    pub unsafe fn get_config_and_visual(
        x_display: *mut xlib::_XDisplay, config: GlConfig,
    ) -> Result<(EglConfig, WindowConfig), GlError> {
        let lib = libloading::Library::new("libEGL.so.1")
            .map_err(|_| GlError::CreationFailed(CreationFailedError::EglInitFailed))?;
        let egl = egl::DynamicInstance::<egl::EGL1_5>::load_required_from(lib)
            .map_err(|_| GlError::CreationFailed(CreationFailedError::EglInitFailed))?;

        let display = egl
            .get_display(x_display as egl::NativeDisplayType)
            .ok_or(GlError::CreationFailed(CreationFailedError::EglInitFailed))?;

        egl.initialize(display)
            .map_err(|_| GlError::CreationFailed(CreationFailedError::EglInitFailed))?;

        egl.bind_api(egl::OPENGL_API)
            .map_err(|_| GlError::CreationFailed(CreationFailedError::EglInitFailed))?;

        let config_attribs = [
            egl::RED_SIZE,
            config.red_bits as egl::Int,
            egl::GREEN_SIZE,
            config.green_bits as egl::Int,
            egl::BLUE_SIZE,
            config.blue_bits as egl::Int,
            egl::ALPHA_SIZE,
            config.alpha_bits as egl::Int,
            egl::DEPTH_SIZE,
            config.depth_bits as egl::Int,
            egl::STENCIL_SIZE,
            config.stencil_bits as egl::Int,
            egl::RENDERABLE_TYPE,
            egl::OPENGL_BIT,
            egl::SURFACE_TYPE,
            egl::WINDOW_BIT,
            egl::NONE,
        ];

        let egl_config = egl
            .choose_first_config(display, &config_attribs)
            .map_err(|_| GlError::CreationFailed(CreationFailedError::NoConfig))?
            .ok_or(GlError::CreationFailed(CreationFailedError::NoConfig))?;

        // Get the native visual ID from the EGL config.
        let visual_id = egl
            .get_config_attrib(display, egl_config, egl::NATIVE_VISUAL_ID)
            .map_err(|_| GlError::CreationFailed(CreationFailedError::NoConfig))?
            as u32;

        // Get depth from X11 for this visual.
        let mut visual_info_template: xlib::XVisualInfo = std::mem::zeroed();
        visual_info_template.visualid = visual_id as u64;
        let mut n_visuals = 0;
        let visual_info = xlib::XGetVisualInfo(
            x_display,
            xlib::VisualIDMask as i64,
            &mut visual_info_template,
            &mut n_visuals,
        );
        let depth = if !visual_info.is_null() && n_visuals > 0 {
            (*visual_info).depth as u8
        } else {
            24
        };
        if !visual_info.is_null() {
            xlib::XFree(visual_info as *mut _);
        }

        Ok((
            EglConfig { gl_config: config, egl_config, egl, display },
            WindowConfig { depth, visual: visual_id },
        ))
    }

    pub unsafe fn make_current(&self) {
        self.egl
            .make_current(self.display, Some(self.surface), Some(self.surface), Some(self.context))
            .expect("eglMakeCurrent failed");
    }

    pub unsafe fn make_not_current(&self) {
        self.egl
            .make_current(self.display, None, None, None)
            .expect("eglMakeCurrent(None) failed");
    }

    pub fn get_proc_address(&self, symbol: &str) -> *const c_void {
        self.egl.get_proc_address(symbol).map_or(std::ptr::null(), |f| f as *const _)
    }

    pub fn swap_buffers(&self) {
        self.egl
            .swap_buffers(self.display, self.surface)
            .expect("eglSwapBuffers failed");
    }
}

impl Drop for GlContext {
    fn drop(&mut self) {
        let _ = self.egl.make_current(self.display, None, None, None);
        let _ = self.egl.destroy_surface(self.display, self.surface);
        let _ = self.egl.destroy_context(self.display, self.context);
        let _ = self.egl.terminate(self.display);
    }
}
