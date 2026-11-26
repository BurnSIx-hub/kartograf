//! Рисование 3D-сцены ([`crate::scene3d`]) в окне Картографа: своя программа GL внутри
//! `PaintCallback` egui. У кадра egui нет глубины, поэтому сцена рисуется в свой кадр —
//! с глубиной и сглаживанием (MSAA), — а готовая картинка копируется в окно.
//!
//! Свет — карта света этажа ([`crate::lighting`]): та же, что ложится на ночной снимок и
//! видна в редакторе по L. Пол и стены берут из неё множитель и сияние в своей точке плана;
//! стена — чуть отступив от себя по нормали, в ту комнату, куда смотрит.

use crate::scene3d::{Alpha, Look, Material, Mesh, Vertex};
use eframe::egui;
use eframe::egui_glow;
use eframe::glow::{self, HasContext};
use std::sync::{Arc, Mutex};

/// Матрица 4×4 по столбцам, как в GL.
pub type Mat4 = [f32; 16];

pub fn mul(a: &Mat4, b: &Mat4) -> Mat4 {
    let mut m = [0.0; 16];
    for c in 0..4 {
        for r in 0..4 {
            m[c * 4 + r] = (0..4).map(|k| a[k * 4 + r] * b[c * 4 + k]).sum();
        }
    }
    m
}

pub fn perspective(fovy: f32, aspect: f32, near: f32, far: f32) -> Mat4 {
    let f = 1.0 / (fovy / 2.0).tan();
    let mut m = [0.0; 16];
    m[0] = f / aspect;
    m[5] = f;
    m[10] = (far + near) / (near - far);
    m[11] = -1.0;
    m[14] = 2.0 * far * near / (near - far);
    m
}

pub fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

pub fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

pub fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

pub fn norm(a: [f32; 3]) -> [f32; 3] {
    let l = dot(a, a).sqrt().max(1e-9);
    [a[0] / l, a[1] / l, a[2] / l]
}

pub fn look_at(eye: [f32; 3], target: [f32; 3], up: [f32; 3]) -> Mat4 {
    let f = norm(sub(target, eye));
    let s = norm(cross(f, up));
    let u = cross(s, f);
    [s[0], u[0], -f[0], 0.0, s[1], u[1], -f[1], 0.0, s[2], u[2], -f[2], 0.0, -dot(s, eye), -dot(u, eye), dot(f, eye), 1.0]
}

/// Узел на месте: сдвиг в `at` и поворот вокруг вертикали, как в glTF (ось x уходит в
/// (cos, 0, −sin)).
pub fn placed(at: [f32; 3], yaw: f32) -> Mat4 {
    let (s, c) = yaw.sin_cos();
    [c, 0.0, -s, 0.0, 0.0, 1.0, 0.0, 0.0, s, 0.0, c, 0.0, at[0], at[1], at[2], 1.0]
}

const VERTEX: &str = r#"#version 330 core
uniform mat4 u_vp;
uniform mat4 u_model;
layout(location = 0) in vec3 a_pos;
layout(location = 1) in vec3 a_n;
layout(location = 2) in vec2 a_uv;
out vec3 v_world;
out vec3 v_n;
out vec2 v_uv;
void main() {
    vec4 w = u_model * vec4(a_pos, 1.0);
    v_world = w.xyz;
    v_n = mat3(u_model) * a_n;
    v_uv = a_uv;
    gl_Position = u_vp * w;
}
"#;

// Стена берёт свет, отступив по нормали: её сторона в комнате — свет комнаты, на улице —
// улицы. Изнанка срезанной стены (видна сверху, когда стены по пояс) — тёмный срез.
const FRAGMENT: &str = r#"#version 330 core
uniform sampler2D u_tex;
uniform sampler2D u_lit;
uniform sampler2D u_glow;
uniform vec4 u_tint;
uniform int u_mode;
uniform vec4 u_light_rect;
uniform vec3 u_sun;
uniform vec3 u_eye;
uniform float u_cut;
uniform int u_caps;
in vec3 v_world;
in vec3 v_n;
in vec2 v_uv;
out vec4 f_color;
void main() {
    if (v_world.y > u_cut) discard;
    vec4 c = texture(u_tex, v_uv) * u_tint;
    if (u_mode == 1 && c.a < 0.5) discard;
    if (u_caps == 1 && !gl_FrontFacing) {
        f_color = vec4(0.10, 0.09, 0.08, 1.0);
        return;
    }
    vec3 n = normalize(v_n);
    if (dot(n, u_eye - v_world) < 0.0) n = -n;
    vec2 p = v_world.xz + n.xz * 0.18;
    vec2 uv = (p - u_light_rect.xy) / u_light_rect.zw;
    vec3 lit = texture(u_lit, uv).rgb;
    vec3 glow = texture(u_glow, uv).rgb;
    float shade = 0.80 + 0.20 * max(dot(n, u_sun), 0.0) - 0.08 * (1.0 - abs(n.y));
    f_color = vec4(c.rgb * lit * shade + glow, u_mode == 2 ? c.a : 1.0);
}
"#;

/// Карта света этажа: угол и шаг в клетках, размер в точках; на точку — множитель по
/// каналам и сияние ([`crate::lighting::preview`]).
#[derive(Clone)]
pub struct LightMap {
    pub origin: [f32; 2],
    pub cell: f32,
    pub w: usize,
    pub h: usize,
    pub data: Vec<[f32; 6]>,
}

/// Кусок карты света этажа, который надо залить заново: `rect` = `[i0, j0, w, h]` точек;
/// пусто — вся карта (новая: другой размер).
pub struct LightUpdate {
    pub storey: usize,
    pub rect: Option<[usize; 4]>,
    pub map: LightMap,
}

/// Что положить на видеокарту: сетки узлов, материалы, картинки.
pub struct Upload {
    pub nodes: Vec<Vec<(usize, Mesh)>>,
    pub materials: Vec<Material>,
    pub images: Vec<image::RgbaImage>,
    pub pawn: Mesh,
}

/// Узел в кадре: номер, матрица, этаж (его карта света), высота среза стен.
#[derive(Clone, Copy)]
pub struct Item {
    pub node: usize,
    pub model: Mat4,
    pub storey: usize,
    pub cut: f32,
    pub caps: bool,
}

/// Что рисовать в этом кадре.
#[derive(Clone, Default)]
pub struct Frame {
    pub vp: Option<Mat4>,
    pub eye: [f32; 3],
    pub sun: [f32; 3],
    pub sky: [f32; 3],
    pub items: Vec<Item>,
    /// Фишка: матрица, этаж, цвет.
    pub pawn: Option<(Mat4, usize, [f32; 3])>,
}

struct Part {
    mat: usize,
    vao: glow::VertexArray,
    vbo: glow::Buffer,
    ibo: glow::Buffer,
    count: i32,
}

struct Target {
    size: [i32; 2],
    msaa: glow::Framebuffer,
    color: glow::Renderbuffer,
    depth: glow::Renderbuffer,
    resolve: glow::Framebuffer,
    resolved: glow::Renderbuffer,
}

struct Uniforms {
    vp: Option<glow::UniformLocation>,
    model: Option<glow::UniformLocation>,
    tex: Option<glow::UniformLocation>,
    lit: Option<glow::UniformLocation>,
    glow: Option<glow::UniformLocation>,
    tint: Option<glow::UniformLocation>,
    mode: Option<glow::UniformLocation>,
    light_rect: Option<glow::UniformLocation>,
    sun: Option<glow::UniformLocation>,
    eye: Option<glow::UniformLocation>,
    cut: Option<glow::UniformLocation>,
    caps: Option<glow::UniformLocation>,
}

struct LightTex {
    lit: glow::Texture,
    glow: glow::Texture,
    rect: [f32; 4],
}

struct Gpu {
    program: glow::Program,
    u: Uniforms,
    /// Сэмплеры: узор плиткой и картинка один раз.
    repeat: glow::Sampler,
    clamp: glow::Sampler,
    linear: glow::Sampler,
    white: glow::Texture,
    black: glow::Texture,
    textures: Vec<glow::Texture>,
    materials: Vec<Material>,
    nodes: Vec<Vec<Part>>,
    pawn: Vec<Part>,
    lights: Vec<Option<LightTex>>,
    target: Option<Target>,
    samples: i32,
}

unsafe fn shader(gl: &glow::Context, kind: u32, text: &str) -> Result<glow::Shader, String> {
    unsafe {
        let s = gl.create_shader(kind)?;
        gl.shader_source(s, text);
        gl.compile_shader(s);
        if gl.get_shader_compile_status(s) { Ok(s) } else { Err(gl.get_shader_info_log(s)) }
    }
}

fn bytes<T>(v: &[T]) -> &[u8] {
    // SAFETY: вершины, номера и числа света — простые числа без промежутков.
    unsafe { std::slice::from_raw_parts(v.as_ptr().cast(), size_of_val(v)) }
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
            let at = |name: &str| gl.get_uniform_location(program, name);
            let u = Uniforms {
                vp: at("u_vp"),
                model: at("u_model"),
                tex: at("u_tex"),
                lit: at("u_lit"),
                glow: at("u_glow"),
                tint: at("u_tint"),
                mode: at("u_mode"),
                light_rect: at("u_light_rect"),
                sun: at("u_sun"),
                eye: at("u_eye"),
                cut: at("u_cut"),
                caps: at("u_caps"),
            };
            let aniso = gl.supported_extensions().iter().any(|e| e.ends_with("texture_filter_anisotropic"));
            let sampler = |wrap: u32, mips: bool| -> Result<glow::Sampler, String> {
                let s = gl.create_sampler()?;
                gl.sampler_parameter_i32(s, glow::TEXTURE_WRAP_S, wrap as i32);
                gl.sampler_parameter_i32(s, glow::TEXTURE_WRAP_T, wrap as i32);
                gl.sampler_parameter_i32(s, glow::TEXTURE_MAG_FILTER, glow::LINEAR as i32);
                gl.sampler_parameter_i32(s, glow::TEXTURE_MIN_FILTER, if mips { glow::LINEAR_MIPMAP_LINEAR } else { glow::LINEAR } as i32);
                if aniso && mips {
                    // TEXTURE_MAX_ANISOTROPY: пол под острым углом не расплывается.
                    gl.sampler_parameter_f32(s, 0x84FE, 8.0);
                }
                Ok(s)
            };
            let (repeat, clamp, linear) = (sampler(glow::REPEAT, true)?, sampler(glow::CLAMP_TO_EDGE, true)?, sampler(glow::CLAMP_TO_EDGE, false)?);
            let one = |c: [u8; 4]| -> Result<glow::Texture, String> {
                let t = gl.create_texture()?;
                gl.bind_texture(glow::TEXTURE_2D, Some(t));
                gl.tex_image_2d(glow::TEXTURE_2D, 0, glow::RGBA8 as i32, 1, 1, 0, glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelUnpackData::Slice(Some(&c)));
                gl.generate_mipmap(glow::TEXTURE_2D);
                Ok(t)
            };
            let (white, black) = (one([255; 4])?, one([0, 0, 0, 255])?);
            let samples = gl.get_parameter_i32(glow::MAX_SAMPLES).clamp(1, 4);
            Ok(Self { program, u, repeat, clamp, linear, white, black, textures: Vec::new(), materials: Vec::new(), nodes: Vec::new(), pawn: Vec::new(), lights: Vec::new(), target: None, samples })
        }
    }

    unsafe fn part(gl: &glow::Context, mat: usize, mesh: &Mesh) -> Result<Part, String> {
        unsafe {
            let (vao, vbo, ibo) = (gl.create_vertex_array()?, gl.create_buffer()?, gl.create_buffer()?);
            gl.bind_vertex_array(Some(vao));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytes(&mesh.v), glow::STATIC_DRAW);
            gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(ibo));
            gl.buffer_data_u8_slice(glow::ELEMENT_ARRAY_BUFFER, bytes(&mesh.idx), glow::STATIC_DRAW);
            let stride = size_of::<Vertex>() as i32;
            gl.enable_vertex_attrib_array(0);
            gl.vertex_attrib_pointer_f32(0, 3, glow::FLOAT, false, stride, 0);
            gl.enable_vertex_attrib_array(1);
            gl.vertex_attrib_pointer_f32(1, 3, glow::FLOAT, false, stride, 12);
            gl.enable_vertex_attrib_array(2);
            gl.vertex_attrib_pointer_f32(2, 2, glow::FLOAT, false, stride, 24);
            gl.bind_vertex_array(None);
            Ok(Part { mat, vao, vbo, ibo, count: mesh.idx.len() as i32 })
        }
    }

    unsafe fn drop_part(gl: &glow::Context, p: &Part) {
        unsafe {
            gl.delete_vertex_array(p.vao);
            gl.delete_buffer(p.vbo);
            gl.delete_buffer(p.ibo);
        }
    }

    /// Убрать сцену: сетки, картинки, карты света.
    unsafe fn clear(&mut self, gl: &glow::Context) {
        unsafe {
            for p in self.nodes.drain(..).flatten().chain(self.pawn.drain(..)) {
                Self::drop_part(gl, &p);
            }
            for t in self.textures.drain(..) {
                gl.delete_texture(t);
            }
            for l in self.lights.drain(..).flatten() {
                gl.delete_texture(l.lit);
                gl.delete_texture(l.glow);
            }
            self.materials.clear();
        }
    }

    unsafe fn destroy(mut self, gl: &glow::Context) {
        unsafe {
            self.clear(gl);
            self.drop_target(gl);
            gl.delete_program(self.program);
            for s in [self.repeat, self.clamp, self.linear] {
                gl.delete_sampler(s);
            }
            gl.delete_texture(self.white);
            gl.delete_texture(self.black);
        }
    }

    unsafe fn upload(&mut self, gl: &glow::Context, up: Upload) -> Result<(), String> {
        unsafe {
            self.clear(gl);
            gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
            for img in &up.images {
                let t = gl.create_texture()?;
                gl.bind_texture(glow::TEXTURE_2D, Some(t));
                gl.tex_image_2d(glow::TEXTURE_2D, 0, glow::RGBA8 as i32, img.width() as i32, img.height() as i32, 0, glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelUnpackData::Slice(Some(img.as_raw())));
                gl.generate_mipmap(glow::TEXTURE_2D);
                self.textures.push(t);
            }
            gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 4);
            for prims in &up.nodes {
                let mut parts = Vec::new();
                for (mat, mesh) in prims {
                    if !mesh.idx.is_empty() {
                        parts.push(Self::part(gl, *mat, mesh)?);
                    }
                }
                self.nodes.push(parts);
            }
            self.materials = up.materials;
            // Фишка — своим материалом в конце списка.
            self.materials.push(Material { look: Look::Color([255, 255, 255]), alpha: Alpha::Opaque, two_sided: false, repeat: false, opacity: 1.0 });
            self.pawn = vec![Self::part(gl, self.materials.len() - 1, &up.pawn)?];
            Ok(())
        }
    }

    unsafe fn light(&mut self, gl: &glow::Context, up: LightUpdate) -> Result<(), String> {
        unsafe {
            let m = &up.map;
            let (lit, glow_c): (Vec<f32>, Vec<f32>) = (m.data.iter().flat_map(|t| [t[0], t[1], t[2]]).collect(), m.data.iter().flat_map(|t| [t[3], t[4], t[5]]).collect());
            while self.lights.len() <= up.storey {
                self.lights.push(None);
            }
            match (&self.lights[up.storey], up.rect) {
                (Some(have), Some([i0, j0, w, h])) => {
                    for (t, data) in [(have.lit, &lit), (have.glow, &glow_c)] {
                        gl.bind_texture(glow::TEXTURE_2D, Some(t));
                        gl.tex_sub_image_2d(glow::TEXTURE_2D, 0, i0 as i32, j0 as i32, w as i32, h as i32, glow::RGB, glow::FLOAT, glow::PixelUnpackData::Slice(Some(bytes(data))));
                    }
                }
                // Кусок без карты (её ещё не залили) — нечего обновлять.
                (None, Some(_)) => {}
                (_, None) => {
                    if let Some(old) = self.lights[up.storey].take() {
                        gl.delete_texture(old.lit);
                        gl.delete_texture(old.glow);
                    }
                    let make = |data: &[f32]| -> Result<glow::Texture, String> {
                        let t = gl.create_texture()?;
                        gl.bind_texture(glow::TEXTURE_2D, Some(t));
                        gl.tex_image_2d(glow::TEXTURE_2D, 0, glow::RGB16F as i32, m.w as i32, m.h as i32, 0, glow::RGB, glow::FLOAT, glow::PixelUnpackData::Slice(Some(bytes(data))));
                        Ok(t)
                    };
                    let rect = [m.origin[0], m.origin[1], m.cell * m.w as f32, m.cell * m.h as f32];
                    self.lights[up.storey] = Some(LightTex { lit: make(&lit)?, glow: make(&glow_c)?, rect });
                }
            }
            Ok(())
        }
    }

    unsafe fn drop_target(&mut self, gl: &glow::Context) {
        unsafe {
            if let Some(t) = self.target.take() {
                gl.delete_framebuffer(t.msaa);
                gl.delete_framebuffer(t.resolve);
                for r in [t.color, t.depth, t.resolved] {
                    gl.delete_renderbuffer(r);
                }
            }
        }
    }

    unsafe fn target(&mut self, gl: &glow::Context, size: [i32; 2]) -> Result<(), String> {
        unsafe {
            if self.target.as_ref().is_some_and(|t| t.size == size) {
                return Ok(());
            }
            self.drop_target(gl);
            let rb = |samples: i32, format: u32| -> Result<glow::Renderbuffer, String> {
                let r = gl.create_renderbuffer()?;
                gl.bind_renderbuffer(glow::RENDERBUFFER, Some(r));
                gl.renderbuffer_storage_multisample(glow::RENDERBUFFER, samples, format, size[0], size[1]);
                Ok(r)
            };
            let (color, depth, resolved) = (rb(self.samples, glow::RGBA8)?, rb(self.samples, glow::DEPTH_COMPONENT24)?, rb(0, glow::RGBA8)?);
            let msaa = gl.create_framebuffer()?;
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(msaa));
            gl.framebuffer_renderbuffer(glow::FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::RENDERBUFFER, Some(color));
            gl.framebuffer_renderbuffer(glow::FRAMEBUFFER, glow::DEPTH_ATTACHMENT, glow::RENDERBUFFER, Some(depth));
            let ok = gl.check_framebuffer_status(glow::FRAMEBUFFER) == glow::FRAMEBUFFER_COMPLETE;
            let resolve = gl.create_framebuffer()?;
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(resolve));
            gl.framebuffer_renderbuffer(glow::FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::RENDERBUFFER, Some(resolved));
            let ok = ok && gl.check_framebuffer_status(glow::FRAMEBUFFER) == glow::FRAMEBUFFER_COMPLETE;
            gl.bind_renderbuffer(glow::RENDERBUFFER, None);
            self.target = Some(Target { size, msaa, color, depth, resolve, resolved });
            if !ok {
                return Err("кадр 3D не собрался".into());
            }
            Ok(())
        }
    }

    unsafe fn draw_parts(&self, gl: &glow::Context, parts: &[Part], item: &Item, tint: Option<[f32; 3]>, blend: bool) {
        unsafe {
            gl.uniform_matrix_4_f32_slice(self.u.model.as_ref(), false, &item.model);
            gl.uniform_1_f32(self.u.cut.as_ref(), item.cut);
            gl.uniform_1_i32(self.u.caps.as_ref(), item.caps as i32);
            let light = self.lights.get(item.storey).and_then(|l| l.as_ref());
            gl.active_texture(glow::TEXTURE1);
            gl.bind_texture(glow::TEXTURE_2D, Some(light.map_or(self.white, |l| l.lit)));
            gl.active_texture(glow::TEXTURE2);
            gl.bind_texture(glow::TEXTURE_2D, Some(light.map_or(self.black, |l| l.glow)));
            let rect = light.map_or([-1e4, -1e4, 2e4, 2e4], |l| l.rect);
            gl.uniform_4_f32(self.u.light_rect.as_ref(), rect[0], rect[1], rect[2], rect[3]);
            for p in parts {
                let Some(m) = self.materials.get(p.mat) else { continue };
                if (m.alpha == Alpha::Blend) != blend {
                    continue;
                }
                let (tex, rgb) = match m.look {
                    Look::Picture(i) => (self.textures.get(i).copied().unwrap_or(self.white), [1.0; 3]),
                    Look::Color(c) => (self.white, c.map(|v| v as f32 / 255.0)),
                };
                let rgb = tint.unwrap_or(rgb);
                gl.active_texture(glow::TEXTURE0);
                gl.bind_texture(glow::TEXTURE_2D, Some(tex));
                gl.bind_sampler(0, Some(if m.repeat { self.repeat } else { self.clamp }));
                gl.uniform_4_f32(self.u.tint.as_ref(), rgb[0], rgb[1], rgb[2], m.opacity);
                gl.uniform_1_i32(self.u.mode.as_ref(), match m.alpha {
                    Alpha::Opaque => 0,
                    Alpha::Mask => 1,
                    Alpha::Blend => 2,
                });
                gl.bind_vertex_array(Some(p.vao));
                gl.draw_elements(glow::TRIANGLES, p.count, glow::UNSIGNED_INT, 0);
            }
        }
    }

    unsafe fn draw(&mut self, gl: &glow::Context, frame: &Frame, info: &egui::PaintCallbackInfo, screen: Option<glow::Framebuffer>) -> Result<(), String> {
        let Some(vp) = frame.vp else { return Ok(()) };
        let view = info.viewport_in_pixels();
        let size = [view.width_px.max(1), view.height_px.max(1)];
        unsafe {
            self.target(gl, size)?;
            let Some(t) = &self.target else { return Ok(()) };
            let (msaa, resolve) = (t.msaa, t.resolve);
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(msaa));
            gl.viewport(0, 0, size[0], size[1]);
            gl.disable(glow::SCISSOR_TEST);
            gl.disable(glow::BLEND);
            gl.disable(glow::CULL_FACE);
            gl.enable(glow::DEPTH_TEST);
            gl.depth_func(glow::LEQUAL);
            gl.depth_mask(true);
            gl.color_mask(true, true, true, true);
            gl.clear_color(frame.sky[0], frame.sky[1], frame.sky[2], 1.0);
            gl.clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);
            gl.use_program(Some(self.program));
            gl.uniform_matrix_4_f32_slice(self.u.vp.as_ref(), false, &vp);
            gl.uniform_3_f32(self.u.sun.as_ref(), frame.sun[0], frame.sun[1], frame.sun[2]);
            gl.uniform_3_f32(self.u.eye.as_ref(), frame.eye[0], frame.eye[1], frame.eye[2]);
            gl.uniform_1_i32(self.u.tex.as_ref(), 0);
            gl.uniform_1_i32(self.u.lit.as_ref(), 1);
            gl.uniform_1_i32(self.u.glow.as_ref(), 2);
            gl.bind_sampler(1, Some(self.linear));
            gl.bind_sampler(2, Some(self.linear));
            for item in &frame.items {
                if let Some(parts) = self.nodes.get(item.node) {
                    self.draw_parts(gl, parts, item, None, false);
                }
            }
            if let Some((model, storey, color)) = frame.pawn {
                let item = Item { node: 0, model, storey, cut: f32::MAX, caps: false };
                self.draw_parts(gl, &self.pawn, &item, Some(color), false);
            }
            // Стёкла — после всего, без записи глубины: сквозь них видно и друг друга.
            gl.enable(glow::BLEND);
            gl.blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
            gl.depth_mask(false);
            for item in &frame.items {
                if let Some(parts) = self.nodes.get(item.node) {
                    self.draw_parts(gl, parts, item, None, true);
                }
            }
            gl.depth_mask(true);
            gl.disable(glow::DEPTH_TEST);
            gl.bind_vertex_array(None);
            for unit in 0..3 {
                gl.bind_sampler(unit, None);
            }
            gl.active_texture(glow::TEXTURE0);
            // Сгладить и перенести в окно, в пределах обрезки egui.
            gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(msaa));
            gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(resolve));
            gl.blit_framebuffer(0, 0, size[0], size[1], 0, 0, size[0], size[1], glow::COLOR_BUFFER_BIT, glow::NEAREST);
            gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(resolve));
            gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, screen);
            let clip = info.clip_rect_in_pixels();
            gl.enable(glow::SCISSOR_TEST);
            gl.scissor(clip.left_px, clip.from_bottom_px, clip.width_px, clip.height_px);
            gl.blit_framebuffer(0, 0, size[0], size[1], view.left_px, view.from_bottom_px, view.left_px + size[0], view.from_bottom_px + size[1], glow::COLOR_BUFFER_BIT, glow::NEAREST);
            gl.bind_framebuffer(glow::FRAMEBUFFER, screen);
        }
        Ok(())
    }
}

/// Общее у окна и рисования: окно кладёт сюда кадр и что залить, рисование берёт.
#[derive(Default)]
struct Shared {
    gpu: Option<Gpu>,
    upload: Option<Upload>,
    lights: Vec<LightUpdate>,
    frame: Frame,
    /// Окно закрыли: при следующем рисовании отдать видеокарте всё.
    release: bool,
    error: Option<String>,
}

/// Рисовальщик 3D-сцены: программа создаётся при первом рисовании — только там есть GL.
#[derive(Clone, Default)]
pub struct Renderer {
    shared: Arc<Mutex<Shared>>,
}

impl Renderer {
    pub fn upload(&self, up: Upload) {
        if let Ok(mut s) = self.shared.lock() {
            s.upload = Some(up);
            s.release = false;
        }
    }

    pub fn light(&self, up: LightUpdate) {
        if let Ok(mut s) = self.shared.lock() {
            // Вся карта этажа заменяет его прежние куски.
            if up.rect.is_none() {
                s.lights.retain(|l| l.storey != up.storey);
            }
            s.lights.push(up);
        }
    }

    /// Что не вышло у видеокарты (старый драйвер, нет памяти).
    pub fn error(&self) -> Option<String> {
        self.shared.lock().ok().and_then(|s| s.error.clone())
    }

    /// Нарисовать кадр в `rect`.
    pub fn paint(&self, painter: &egui::Painter, rect: egui::Rect, frame: Frame) {
        if let Ok(mut s) = self.shared.lock() {
            s.frame = frame;
        }
        let shared = self.shared.clone();
        let callback = egui_glow::CallbackFn::new(move |info, gl_painter| {
            let gl = gl_painter.gl();
            let Ok(mut s) = shared.lock() else { return };
            if s.error.is_some() {
                return;
            }
            let result = unsafe { Self::step(gl, &mut s, &info, gl_painter.intermediate_fbo()) };
            if let Err(e) = result {
                eprintln!("3D: {e}");
                s.error = Some(e);
            }
        });
        painter.add(egui::PaintCallback { rect, callback: Arc::new(callback) });
    }

    unsafe fn step(gl: &glow::Context, s: &mut Shared, info: &egui::PaintCallbackInfo, screen: Option<glow::Framebuffer>) -> Result<(), String> {
        unsafe {
            if s.release {
                if let Some(g) = s.gpu.take() {
                    g.destroy(gl);
                }
                s.release = false;
                return Ok(());
            }
            if s.gpu.is_none() {
                s.gpu = Some(Gpu::new(gl)?);
            }
            let Some(g) = s.gpu.as_mut() else { return Ok(()) };
            if let Some(up) = s.upload.take() {
                g.upload(gl, up)?;
            }
            for up in std::mem::take(&mut s.lights) {
                g.light(gl, up)?;
            }
            let frame = std::mem::take(&mut s.frame);
            let Some(g) = s.gpu.as_mut() else { return Ok(()) };
            let r = g.draw(gl, &frame, info, screen);
            s.frame = frame;
            r
        }
    }

    /// Окно закрыто: отдать видеокарте память. Нужен ещё один вызов рисования — `paint_release`.
    pub fn release(&self, painter: &egui::Painter) {
        let has = self.shared.lock().is_ok_and(|s| s.gpu.is_some());
        if !has {
            return;
        }
        if let Ok(mut s) = self.shared.lock() {
            s.release = true;
            s.upload = None;
            s.lights.clear();
            s.frame = Frame::default();
        }
        // Рисование в точку в углу: egui зовёт только вызовы с ненулевым прямоугольником.
        let shared = self.shared.clone();
        let callback = egui_glow::CallbackFn::new(move |_info, gl_painter| {
            if let Ok(mut s) = shared.lock()
                && s.release
            {
                if let Some(g) = s.gpu.take() {
                    unsafe { g.destroy(gl_painter.gl()) };
                }
                s.release = false;
            }
        });
        let at = painter.clip_rect().min;
        painter.add(egui::PaintCallback { rect: egui::Rect::from_min_size(at, egui::vec2(1.0, 1.0)), callback: Arc::new(callback) });
    }
}

/// Фишка: подставка, тело и голова, выше пояса клетки; низ — в нуле, середина — на оси.
pub fn pawn() -> Mesh {
    let mut m = Mesh::default();
    // Тело вращения по профилю (радиус, высота), снизу вверх.
    let profile: [[f32; 2]; 10] = [[0.0, 0.0], [0.36, 0.0], [0.38, 0.03], [0.36, 0.08], [0.24, 0.10], [0.20, 0.30], [0.15, 0.72], [0.17, 0.78], [0.13, 0.84], [0.0, 0.84]];
    let seg = 24;
    let lathe = |m: &mut Mesh, prof: &[[f32; 2]]| {
        let base = m.v.len() as u32;
        for (i, p) in prof.iter().enumerate() {
            // Нормаль профиля — поперёк отрезка между соседями.
            let (a, b) = (prof[i.saturating_sub(1)], prof[(i + 1).min(prof.len() - 1)]);
            let (dr, dy) = (b[0] - a[0], b[1] - a[1]);
            let l = dr.hypot(dy).max(1e-6);
            let (nr, ny) = (dy / l, -dr / l);
            for k in 0..=seg {
                let t = k as f32 / seg as f32 * std::f32::consts::TAU;
                let (s, c) = t.sin_cos();
                m.v.push(Vertex { p: [p[0] * c, p[1], p[0] * s], n: [nr * c, ny, nr * s], uv: [0.0, 0.0] });
            }
        }
        let row = seg + 1;
        for i in 0..prof.len() as u32 - 1 {
            for k in 0..seg {
                let (a, b) = (base + i * row + k, base + (i + 1) * row + k);
                m.idx.extend_from_slice(&[a, b, a + 1, a + 1, b, b + 1]);
            }
        }
    };
    lathe(&mut m, &profile);
    // Голова — шар.
    let head: Vec<[f32; 2]> = (0..=10).map(|i| {
        let t = std::f32::consts::PI * (i as f32 / 10.0 - 0.5);
        [0.16 * t.cos(), 0.98 + 0.16 * t.sin()]
    }).collect();
    lathe(&mut m, &head);
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(m: &Mat4, p: [f32; 3]) -> [f32; 4] {
        let v = [p[0], p[1], p[2], 1.0];
        let mut out = [0.0; 4];
        for (r, o) in out.iter_mut().enumerate() {
            *o = (0..4).map(|c| m[c * 4 + r] * v[c]).sum();
        }
        out
    }

    #[test]
    fn a_node_turns_like_gltf() {
        // Ось x узла с поворотом на четверть оборота уходит в (0, 0, −1).
        let p = apply(&placed([1.0, 2.0, 3.0], std::f32::consts::FRAC_PI_2), [1.0, 0.0, 0.0]);
        assert!((p[0] - 1.0).abs() < 1e-5 && (p[1] - 2.0).abs() < 1e-5 && (p[2] - 2.0).abs() < 1e-5, "{p:?}");
    }

    #[test]
    fn the_camera_sees_its_target_in_the_middle() {
        let vp = mul(&perspective(0.9, 1.5, 0.1, 100.0), &look_at([0.0, 10.0, 10.0], [3.0, 0.0, 2.0], [0.0, 1.0, 0.0]));
        let c = apply(&vp, [3.0, 0.0, 2.0]);
        assert!((c[0] / c[3]).abs() < 1e-4 && (c[1] / c[3]).abs() < 1e-4, "{c:?}");
        // Правее цели — правее на экране.
        let r = apply(&vp, [4.0, 0.0, 2.0]);
        assert!(r[0] / r[3] > 0.0);
    }

    #[test]
    fn the_pawn_stands_on_the_floor_and_is_shorter_than_a_door() {
        let m = pawn();
        let (lo, hi) = m.v.iter().fold((f32::MAX, f32::MIN), |(lo, hi), v| (lo.min(v.p[1]), hi.max(v.p[1])));
        assert!(lo.abs() < 1e-6, "низ {lo}");
        assert!(hi > 1.0 && hi < crate::scene3d::DOOR, "верх {hi}");
        assert!(m.idx.iter().all(|&i| (i as usize) < m.v.len()));
    }
}
