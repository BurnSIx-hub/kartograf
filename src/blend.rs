//! Край покрытия по рисунку. Обычный мягкий край делает камень или лист полупрозрачным
//! призраком. Здесь сила мазка сдвигается светлотой текстуры относительно её окрестности:
//! выступающее (камень, лист) держится дольше, а щели между ним уходят первыми, и край
//! идёт по очертаниям рисунка. Ровная текстура (песок, вода) сохраняет обычный мягкий край.
//!
//! В шейдерах egui этого не сделать, поэтому слой рисуется своей программой через
//! `PaintCallback`, в том же порядке, что и остальные фигуры. Попадает он и в выгрузку.

use eframe::egui::{self, Shape};
use eframe::egui_glow;
use eframe::glow::{self, HasContext};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

const VERTEX: &str = r#"#version 330 core
uniform vec2 u_screen;
uniform vec2 u_origin;
layout(location = 0) in vec2 a_pos;
layout(location = 1) in vec2 a_tc;
layout(location = 2) in vec4 a_color;
out vec2 v_tc;
out float v_alpha;
void main() {
    vec2 p = (a_pos - u_origin) / u_screen;
    gl_Position = vec4(p.x * 2.0 - 1.0, 1.0 - p.y * 2.0, 0.0, 1.0);
    v_tc = a_tc;
    v_alpha = a_color.a;
}
"#;

// `textureLod` с пятого уровня — средняя светлота вокруг точки. Мелкая рябь не считается:
// волны песка иначе рвут край полосами. Сдвиг сильнее всего на середине силы и сходит
// на нет к 0 и 1: сплошная заливка остаётся сплошной.
const FRAGMENT: &str = r#"#version 330 core
uniform sampler2D u_tex;
in vec2 v_tc;
in float v_alpha;
out vec4 f_color;
const vec3 LUMA = vec3(0.3, 0.59, 0.11);
void main() {
    vec4 c = texture(u_tex, v_tc);
    float near = dot(textureLod(u_tex, v_tc, 5.0).rgb, LUMA);
    float e = dot(c.rgb, LUMA) - near;
    float d = sign(e) * smoothstep(0.05, 0.16, abs(e));
    float a = clamp(v_alpha + d * min(v_alpha, 1.0 - v_alpha) * 4.0, 0.0, 1.0);
    f_color = c * a;
}
"#;

struct Gpu {
    program: glow::Program,
    vao: glow::VertexArray,
    vertices: glow::Buffer,
    indices: glow::Buffer,
    screen: Option<glow::UniformLocation>,
    origin: Option<glow::UniformLocation>,
}

unsafe fn shader(gl: &glow::Context, kind: u32, text: &str) -> Result<glow::Shader, String> {
    unsafe {
        let s = gl.create_shader(kind)?;
        gl.shader_source(s, text);
        gl.compile_shader(s);
        if gl.get_shader_compile_status(s) { Ok(s) } else { Err(gl.get_shader_info_log(s)) }
    }
}

impl Gpu {
    unsafe fn new(gl: &glow::Context) -> Result<Self, String> {
        unsafe {
            let (vs, fs) = (shader(gl, glow::VERTEX_SHADER, VERTEX)?, shader(gl, glow::FRAGMENT_SHADER, FRAGMENT)?);
            let program = gl.create_program()?;
            gl.attach_shader(program, vs);
            gl.attach_shader(program, fs);
            gl.link_program(program);
            gl.delete_shader(vs);
            gl.delete_shader(fs);
            if !gl.get_program_link_status(program) {
                return Err(gl.get_program_info_log(program));
            }
            let (vao, vertices, indices) = (gl.create_vertex_array()?, gl.create_buffer()?, gl.create_buffer()?);
            gl.bind_vertex_array(Some(vao));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vertices));
            gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(indices));
            let stride = size_of::<egui::epaint::Vertex>() as i32;
            gl.enable_vertex_attrib_array(0);
            gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, stride, 0);
            gl.enable_vertex_attrib_array(1);
            gl.vertex_attrib_pointer_f32(1, 2, glow::FLOAT, false, stride, 8);
            gl.enable_vertex_attrib_array(2);
            gl.vertex_attrib_pointer_f32(2, 4, glow::UNSIGNED_BYTE, true, stride, 16);
            gl.bind_vertex_array(None);
            let (screen, origin) = (gl.get_uniform_location(program, "u_screen"), gl.get_uniform_location(program, "u_origin"));
            Ok(Self { program, vao, vertices, indices, screen, origin })
        }
    }

    unsafe fn draw(&self, gl: &glow::Context, mesh: &egui::Mesh, texture: glow::Texture, info: &egui::PaintCallbackInfo) {
        let vp = info.viewport_in_pixels();
        let ppp = info.pixels_per_point;
        unsafe {
            let raw = |p: *const u8, n: usize| std::slice::from_raw_parts(p, n);
            gl.use_program(Some(self.program));
            gl.uniform_2_f32(self.screen.as_ref(), vp.width_px as f32 / ppp, vp.height_px as f32 / ppp);
            gl.uniform_2_f32(self.origin.as_ref(), vp.left_px as f32 / ppp, vp.top_px as f32 / ppp);
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            gl.bind_vertex_array(Some(self.vao));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vertices));
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, raw(mesh.vertices.as_ptr().cast(), size_of_val(mesh.vertices.as_slice())), glow::STREAM_DRAW);
            gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(self.indices));
            gl.buffer_data_u8_slice(glow::ELEMENT_ARRAY_BUFFER, raw(mesh.indices.as_ptr().cast(), size_of_val(mesh.indices.as_slice())), glow::STREAM_DRAW);
            gl.draw_elements(glow::TRIANGLES, mesh.indices.len() as i32, glow::UNSIGNED_INT, 0);
            gl.bind_vertex_array(None);
        }
    }
}

/// Программа создаётся при первом рисовании: только там есть контекст GL.
#[derive(Clone, Default)]
pub struct Blend {
    gpu: Arc<Mutex<Option<Gpu>>>,
    /// Шейдер не собрался (старый драйвер): слои идут обычным мягким краем.
    broken: Arc<AtomicBool>,
}

impl Blend {
    /// Нарисовать текстурный слой с краем по рисунку.
    pub fn paint(&self, painter: &egui::Painter, mesh: egui::Mesh) {
        if self.broken.load(Ordering::Relaxed) {
            painter.add(Shape::mesh(mesh));
            return;
        }
        let this = self.clone();
        let callback = egui_glow::CallbackFn::new(move |info, gl_painter| {
            let gl = gl_painter.gl();
            let Ok(mut gpu) = this.gpu.lock() else { return };
            if gpu.is_none() {
                match unsafe { Gpu::new(gl) } {
                    Ok(g) => *gpu = Some(g),
                    Err(e) => {
                        eprintln!("шейдер края не собрался: {e}");
                        this.broken.store(true, Ordering::Relaxed);
                        return;
                    }
                }
            }
            if let (Some(g), Some(texture)) = (gpu.as_ref(), gl_painter.texture(mesh.texture_id)) {
                unsafe { g.draw(gl, &mesh, texture, &info) };
            }
        });
        painter.add(egui::PaintCallback { rect: painter.clip_rect(), callback: Arc::new(callback) });
    }
}
