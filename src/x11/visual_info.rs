use crate::x11::xcb_connection::XcbConnection;
use std::error::Error;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    Colormap, ColormapAlloc, ConnectionExt, Screen, VisualClass, Visualid,
};
use x11rb::COPY_FROM_PARENT;

/// Stored GL config for creating the context after the window exists.
#[cfg(all(any(feature = "opengl", feature = "egl"), not(feature = "egl")))]
pub type GlFbConfig = crate::gl::x11::FbConfig;

#[cfg(feature = "egl")]
pub type GlFbConfig = crate::gl::egl_x11::EglConfig;

pub(super) struct WindowVisualConfig {
    #[cfg(any(feature = "opengl", feature = "egl"))]
    pub fb_config: Option<GlFbConfig>,

    pub visual_depth: u8,
    pub visual_id: Visualid,
    pub color_map: Option<Colormap>,
}

// TODO: make visual negotiation actually check all of a visual's parameters
impl WindowVisualConfig {
    #[cfg(any(feature = "opengl", feature = "egl"))]
    pub fn find_best_visual_config_for_gl(
        connection: &XcbConnection, gl_config: Option<crate::gl::GlConfig>,
    ) -> Result<Self, Box<dyn Error>> {
        let Some(gl_config) = gl_config else { return Self::find_best_visual_config(connection) };

        #[cfg(not(feature = "egl"))]
        let (fb_config, window_config) = unsafe {
            crate::gl::platform::GlContext::get_fb_config_and_visual(connection.dpy, gl_config)
        }
        .expect("Could not fetch framebuffer config");

        #[cfg(feature = "egl")]
        let (fb_config, window_config) = unsafe {
            crate::gl::platform::GlContext::get_config_and_visual(connection.dpy, gl_config)
        }
        .expect("Could not fetch EGL config");

        Ok(Self {
            fb_config: Some(fb_config),
            visual_depth: window_config.depth,
            visual_id: window_config.visual,
            color_map: Some(create_color_map(connection, window_config.visual)?),
        })
    }

    pub fn find_best_visual_config(connection: &XcbConnection) -> Result<Self, Box<dyn Error>> {
        match find_visual_for_depth(connection.screen(), 32) {
            None => Ok(Self::copy_from_parent()),
            Some(visual_id) => Ok(Self {
                #[cfg(any(feature = "opengl", feature = "egl"))]
                fb_config: None,
                visual_id,
                visual_depth: 32,
                color_map: Some(create_color_map(connection, visual_id)?),
            }),
        }
    }

    const fn copy_from_parent() -> Self {
        Self {
            #[cfg(any(feature = "opengl", feature = "egl"))]
            fb_config: None,
            visual_depth: COPY_FROM_PARENT as u8,
            visual_id: COPY_FROM_PARENT,
            color_map: None,
        }
    }
}

// For this 32-bit depth to work, you also need to define a color map and set a border
// pixel: https://cgit.freedesktop.org/xorg/xserver/tree/dix/window.c#n818
fn create_color_map(
    connection: &XcbConnection, visual_id: Visualid,
) -> Result<Colormap, Box<dyn Error>> {
    let colormap = connection.conn.generate_id()?;
    connection.conn.create_colormap(
        ColormapAlloc::NONE,
        colormap,
        connection.screen().root,
        visual_id,
    )?;

    Ok(colormap)
}

fn find_visual_for_depth(screen: &Screen, depth: u8) -> Option<Visualid> {
    for candidate_depth in &screen.allowed_depths {
        if candidate_depth.depth != depth {
            continue;
        }

        for candidate_visual in &candidate_depth.visuals {
            if candidate_visual.class == VisualClass::TRUE_COLOR {
                return Some(candidate_visual.visual_id);
            }
        }
    }

    None
}
