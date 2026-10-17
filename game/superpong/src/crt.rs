//! Classic video: the CRT post-process of SPEC §13.1.
//!
//! One shader pass draws the native picture texture (four sub-columns per
//! clock, one row per line) into the picture rectangle as a 1970s colour TV
//! showed it: composite-video luma and chroma filters, Gaussian scanlines
//! that widen with brightness, a phosphor mask, a little glow, barrel
//! curvature with a dark tube face, and a vignette. The GLSL is in
//! `shaders/crt.{vert,frag}`; it builds on Timothy Lottes' public-domain
//! CRT shader (credited in `crt.frag`).
//!
//! The shader is GLSL 1.00, which macroquad's OpenGL backend compiles on
//! macOS, Linux and Windows. This app never asks for the Metal backend; if
//! the shader cannot be built there or anywhere else, [`Crt::load`] reports
//! why and the renderer falls back to the pixel picture.

use macroquad::miniquad::{UniformDesc, UniformType};
use macroquad::prelude::*;

const VERTEX: &str = include_str!("shaders/crt.vert");
const FRAGMENT: &str = include_str!("shaders/crt.frag");

/// The compiled CRT material.
pub struct Crt {
    material: Material,
}

impl Crt {
    /// Compiles the shader. The error is the GL compiler's message.
    pub fn load() -> Result<Self, String> {
        let params = MaterialParams {
            uniforms: vec![
                UniformDesc::new("SourceSize", UniformType::Float2),
                UniformDesc::new("OutputSize", UniformType::Float2),
            ],
            ..Default::default()
        };
        load_material(ShaderSource::Glsl { vertex: VERTEX, fragment: FRAGMENT }, params)
            .map(|material| Self { material })
            .map_err(|e| e.to_string())
    }

    /// Draws `picture` through the shader into the rectangle at (`x`, `y`)
    /// of `w` x `h` logical units, which is `device` pixels on screen.
    pub fn draw(&self, picture: &Texture2D, (x, y, w, h): (f32, f32, f32, f32), device: (i32, i32)) {
        self.material.set_uniform("SourceSize", picture.size());
        self.material.set_uniform("OutputSize", vec2(device.0 as f32, device.1 as f32));
        gl_use_material(&self.material);
        draw_texture_ex(
            picture,
            x,
            y,
            WHITE,
            DrawTextureParams { dest_size: Some(vec2(w, h)), ..Default::default() },
        );
        gl_use_default_material();
    }
}
