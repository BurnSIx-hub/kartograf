//! Тестовая зона: окно 3D в Картографе. Карта снимается так же, как для `.glb` в выгрузку
//! (пол без стен на каждый этаж), из снимка строится та же сцена [`crate::scene3d`], и окно
//! её рисует ([`crate::render3d`]): облёт камерой, фишка ходит со столкновениями о стены и
//! закрытые двери, двери открываются, свет огней — как на ночном снимке и в Foundry.
//!
//! Окно — снимок карты: правки на карте видны после «Снять заново».

use super::*;
use crate::lighting::{self, Time};
use crate::render3d::{self, Frame, Item, LightMap, LightUpdate, Renderer, Upload};
use crate::scene3d::{self, Part, STOREY};
use super::props3d;
use std::collections::HashMap;

/// Длинная сторона картинки пола в окне, точки: больше — память видеокарты без пользы.
const FLOOR_SIDE: f32 = 4096.0;
/// Точек карты света на длинную сторону, не больше.
const LIGHT_SIDE: f32 = 1024.0;
/// Угол обзора по вертикали.
const FOV: f32 = 0.87;
/// Фишка: радиус в клетках (средний рост, клетка 5 футов), скорость шага, клеток в секунду.
const TOKEN_R: f32 = 0.3;
const SPEED: f32 = 4.0;
/// Факел у фишки: яркий и тусклый свет, клетки (20 и 40 футов, как в D&D).
const TORCH: (f32, f32) = (4.0, 8.0);
/// Дверь открывается фишкой отсюда и щелчком у самой двери, клетки.
const REACH: f32 = 1.6;
const CLICK_REACH: f32 = 0.8;
/// Створка поворачивается за столько секунд.
const SWING_TIME: f32 = 0.45;
/// Стены по пояс: выше этого над полом этажа срезаны.
const CUT: f32 = 0.9;
/// Размытие карты света — столько точек с краю куска считаются заново ([`lighting::preview`]).
const BLUR_PAD: usize = 4;

/// Камера: смотрит на `target` с расстояния `dist`, повёрнута на `yaw` (0 — с юга) и
/// поднята на `pitch`.
#[derive(Clone, Copy)]
struct Orbit {
    target: [f32; 3],
    yaw: f32,
    pitch: f32,
    dist: f32,
}

impl Orbit {
    fn eye(&self) -> [f32; 3] {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        [self.target[0] + self.dist * sy * cp, self.target[1] + self.dist * sp, self.target[2] + self.dist * cy * cp]
    }

    /// Вперёд и вправо по земле — куда идёт фишка по W и D.
    fn ground(&self) -> ([f32; 2], [f32; 2]) {
        let (s, c) = self.yaw.sin_cos();
        ([-s, -c], [c, -s])
    }
}

struct NodeInfo {
    part: Part,
    storey: usize,
    at: [f32; 3],
    yaw: f32,
    /// Створка: дверь (без `_a`/`_b` двойной) и на сколько поворачивается.
    door: Option<(String, f32)>,
    /// Образец предмета (модель или плоская картинка): сам не рисуется, только местами из
    /// [`View3d::props`].
    template: bool,
}

/// Предмет на месте: образец, матрица, этаж; крона прячется, когда стены по пояс; `bias` —
/// порядок плоской картинки на полу (сдвиг глубины).
struct PropAt {
    node: usize,
    model: render3d::Mat4,
    storey: usize,
    crown: bool,
    bias: f32,
}

/// Плоская картинка предмета (без модели) в окне 3D — не больше стольких точек по стороне.
const CARD_SIDE: u32 = 384;
/// Высота плоских картинок над полом этажа: навес и бельё, крона.
const MID_Y: f32 = 1.5;
const CROWN_Y: f32 = 2.4;

struct Door3 {
    seg: Segment,
    id: String,
    /// Номер преграды света этой двери в `lighting::Scene::blockers`.
    blocker: usize,
}

struct Floor3 {
    /// Полутолщина стен этажа.
    half: f32,
    /// Не пройти: стены, низкие стены, кромки обрывов, окна.
    solid: Vec<Segment>,
    doors: Vec<Door3>,
    light: lighting::Scene,
    map: Option<LightMap>,
}

#[derive(Clone, Copy, Default)]
struct DoorState {
    open: bool,
    /// Насколько открыта сейчас, 0..1.
    t: f32,
}

#[derive(Clone, Copy, PartialEq)]
struct Token {
    at: [f32; 2],
    storey: usize,
}

pub(super) struct View3d {
    pub open: bool,
    renderer: Renderer,
    nodes: Vec<NodeInfo>,
    floors: Vec<Floor3>,
    size: [f32; 2],
    cam: Orbit,
    token: Option<Token>,
    doors: HashMap<String, DoorState>,
    time: Time,
    /// Верхний видимый этаж: выше — сняты, как крыша.
    top: usize,
    cut: bool,
    torch: bool,
    /// Камера идёт за фишкой.
    follow: bool,
    /// Где сейчас горит факел фишки: место и этаж.
    torch_at: Option<([f32; 2], usize)>,
    /// Карта, с которой снято: правки после — «Снять заново».
    rev: u64,
    /// Сколько ушло на снимок и сборку, для подписи.
    took: f32,
    /// Сколько кадров нарисовано: снимок окна (`--shot`) ждёт, пока сцена ляжет.
    painted: u32,
    /// Где вид на экране (прошлый кадр).
    rect: Rect,
    /// Предметы, что в 3D не в картинке пола: моделью или плоской картинкой.
    props: Vec<PropAt>,
    /// Сколько из них моделями.
    modelled: usize,
}

/// Что видно снаружи окна 3D — самопроверке.
pub struct Probe3d {
    pub rect: Rect,
    pub yaw: f32,
    pub token: Option<[f32; 2]>,
    /// Сколько дверей открыто.
    pub open: usize,
    pub painted: u32,
    pub error: Option<String>,
    /// Предметов в 3D и сколько из них моделями.
    pub props: usize,
    pub modelled: usize,
}

fn id() -> egui::Id {
    egui::Id::new("view3d")
}

fn closest(p: [f32; 2], s: Segment) -> [f32; 2] {
    let (a, b) = s;
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let l2 = dx * dx + dy * dy;
    let t = if l2 < 1e-12 { 0.0 } else { (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / l2).clamp(0.0, 1.0) };
    [a[0] + t * dx, a[1] + t * dy]
}

fn segments(lines: &[Vec<[f32; 2]>]) -> impl Iterator<Item = Segment> + '_ {
    lines.iter().flat_map(|l| l.windows(2).map(|w| (w[0], w[1])))
}

/// Вытолкнуть круг радиуса `r` с середины `p` из отрезков `segs` толщиной `2·half`.
fn push_out<'a>(mut p: [f32; 2], r: f32, segs: impl Iterator<Item = (Segment, f32)> + Clone + 'a) -> [f32; 2] {
    for _ in 0..3 {
        let mut moved = false;
        for (s, half) in segs.clone() {
            let reach = r + half;
            let q = closest(p, s);
            let (dx, dy) = (p[0] - q[0], p[1] - q[1]);
            let d = dx.hypot(dy);
            if d >= reach {
                continue;
            }
            // Ровно на оси отрезка — в сторону его левой нормали.
            let (nx, ny) = if d > 1e-6 {
                (dx / d, dy / d)
            } else {
                let (ex, ey) = (s.1[0] - s.0[0], s.1[1] - s.0[1]);
                let l = ex.hypot(ey).max(1e-6);
                (ey / l, -ex / l)
            };
            p = [q[0] + nx * reach, q[1] + ny * reach];
            moved = true;
        }
        if !moved {
            break;
        }
    }
    p
}

/// Время суток по свету сцены, выбранному на карте.
fn time_of(map: &Map) -> Time {
    match export::scene_light(map) {
        (true, d) if d <= 0.0 => Time::Day,
        (true, _) => Time::Dusk,
        _ => Time::Night,
    }
}

impl Floor3 {
    /// Этаж `k` по разбору для Foundry: стены, двери (их метки — как у створок сцены, по
    /// точкам `px` на клетку) и свет.
    fn new(k: usize, lay: &export::Layout, rooms: Mask, half: f32, px: f32) -> Floor3 {
        let light = lighting::Scene::new(lay, rooms, Vec::new());
        // Преграды света — сначала отрезки стен, потом двери и окна по порядку.
        let walls_n: usize = lay.walls.iter().map(|w| w.len().saturating_sub(1)).sum();
        let at = |v: [f32; 2]| [(v[0] * px).round() as i64, (v[1] * px).round() as i64];
        let mut solid: Vec<Segment> = segments(&lay.walls).chain(segments(&lay.low)).chain(segments(&lay.cliffs)).collect();
        let mut doors = Vec::new();
        for (i, po) in lay.portals.iter().enumerate() {
            if po.kind.is_door() || po.kind == PortalKind::Secret {
                let (a, b) = (at(po.seg.0), at(po.seg.1));
                doors.push(Door3 { seg: po.seg, id: format!("{k}:{},{},{},{}", a[0], a[1], b[0], b[1]), blocker: walls_n + i });
            } else {
                solid.push(po.seg);
            }
        }
        Floor3 { half, solid, doors, light, map: None }
    }
}

impl View3d {
    fn fresh(renderer: Renderer, nodes: Vec<NodeInfo>, floors: Vec<Floor3>, size: [f32; 2], time: Time) -> View3d {
        let span = size[0].max(size[1]);
        View3d {
            open: true,
            renderer,
            nodes,
            floors,
            size,
            cam: Orbit { target: [size[0] / 2.0, 0.0, size[1] / 2.0], yaw: 0.0, pitch: 0.95, dist: span * 0.9 },
            token: None,
            doors: HashMap::new(),
            time,
            top: 0,
            cut: false,
            torch: true,
            follow: false,
            torch_at: None,
            rev: 0,
            took: 0.0,
            painted: 0,
            rect: Rect::NOTHING,
            props: Vec::new(),
            modelled: 0,
        }
    }

    /// Взять у прежнего окна камеру, время, вид, фишку и открытые двери, что остались на месте.
    fn carry(&mut self, old: View3d) {
        let n = self.floors.len();
        self.cam = old.cam;
        self.time = old.time;
        self.top = old.top.min(n.saturating_sub(1));
        self.cut = old.cut;
        self.torch = old.torch;
        self.follow = old.follow;
        self.token = old.token.filter(|t| t.storey < n);
        for f in &mut self.floors {
            for d in &f.doors {
                if let Some(st) = old.doors.get(&d.id).filter(|s| s.open) {
                    self.doors.insert(d.id.clone(), *st);
                    if let Some(b) = f.light.blockers.get_mut(d.blocker) {
                        b.pass = 1.0;
                    }
                }
            }
        }
    }

    /// Сцена для снимка (`--view3d --shot`, переменная `KARTOGRAF_VIEW3D`): через «;» —
    /// `day`/`dusk`/`night`, `cut`, `notorch`, `open` (все двери), `token=x,y[,этаж]`,
    /// `cam=поворот°,наклон°,даль`, `look=x,y`.
    fn preset(&mut self, text: &str) {
        let nums = |v: &str| v.split(',').filter_map(|x| x.trim().parse::<f32>().ok()).collect::<Vec<_>>();
        for part in text.split(';').map(str::trim) {
            let (key, val) = part.split_once('=').unwrap_or((part, ""));
            match key {
                "day" => self.time = Time::Day,
                "dusk" => self.time = Time::Dusk,
                "night" => self.time = Time::Night,
                "cut" => self.cut = true,
                "notorch" => self.torch = false,
                "open" => {
                    let all: Vec<(usize, usize)> = self.floors.iter().enumerate().flat_map(|(k, f)| (0..f.doors.len()).map(move |i| (k, i))).collect();
                    for (k, i) in all {
                        self.toggle(k, i);
                        if let Some(d) = self.doors.get_mut(&self.floors[k].doors[i].id) {
                            d.t = 1.0;
                        }
                    }
                }
                "token" => {
                    let v = nums(val);
                    if v.len() >= 2 {
                        let storey = v.get(2).map_or(0, |&s| (s as usize).min(self.floors.len().saturating_sub(1)));
                        self.token = Some(Token { at: [v[0], v[1]], storey });
                        self.top = storey;
                        self.walk([0.0, 1e-4]);
                        self.follow = true;
                        if let Some(t) = self.token {
                            self.cam.target = [t.at[0], Self::floor_y(t.storey) + 0.6, t.at[1]];
                        }
                    }
                }
                "cam" => {
                    let v = nums(val);
                    if v.len() >= 3 {
                        self.cam.yaw = v[0].to_radians();
                        self.cam.pitch = v[1].to_radians().clamp(0.06, 1.53);
                        self.cam.dist = v[2].clamp(1.2, 600.0);
                    }
                }
                "look" => {
                    let v = nums(val);
                    if v.len() >= 2 {
                        self.cam.target[0] = v[0];
                        self.cam.target[2] = v[1];
                        self.follow = false;
                    }
                }
                _ => {}
            }
        }
    }

    fn floor_y(k: usize) -> f32 {
        k as f32 * STOREY
    }

    /// Пересчитать карту света этажа `k` целиком (`area` пусто) или в рамке `area` клеток.
    fn relight(&mut self, k: usize, area: Option<[f32; 4]>) {
        let Some(f) = self.floors.get_mut(k) else { return };
        if self.time == Time::Day {
            // Днём свет ровный: одна точка на всю карту.
            let map = LightMap { origin: [-1e4, -1e4], cell: 2e4, w: 1, h: 1, data: vec![[1.0, 1.0, 1.0, 0.0, 0.0, 0.0]] };
            f.map = None;
            self.renderer.light(LightUpdate { storey: k, rect: None, map });
            return;
        }
        let cell = (self.size[0].max(self.size[1]) / LIGHT_SIDE).max(1.0 / lighting::PER_CELL);
        let (w, h) = ((self.size[0] / cell).ceil().max(1.0) as usize, (self.size[1] / cell).ceil().max(1.0) as usize);
        let whole = f.map.as_ref().is_none_or(|m| m.w != w || m.h != h || m.cell != cell);
        match area {
            Some(a) if !whole => {
                let Some(m) = f.map.as_mut() else { return };
                let span = |v: f32, n: usize| ((v / cell).floor().max(0.0) as usize).min(n);
                let (i0, i1) = (span(a[0], w), (span(a[2], w) + 1).min(w));
                let (j0, j1) = (span(a[1], h), (span(a[3], h) + 1).min(h));
                if i1 <= i0 || j1 <= j0 {
                    return;
                }
                let (oi0, oj0) = (i0.saturating_sub(BLUR_PAD), j0.saturating_sub(BLUR_PAD));
                let (oi1, oj1) = ((i1 + BLUR_PAD).min(w), (j1 + BLUR_PAD).min(h));
                let (ow, oh) = (oi1 - oi0, oj1 - oj0);
                let sub = lighting::preview(&f.light, self.time, [oi0 as f32 * cell, oj0 as f32 * cell], cell, ow, oh);
                let (iw, ih) = (i1 - i0, j1 - j0);
                let mut part = Vec::with_capacity(iw * ih);
                for j in j0..j1 {
                    let row = &sub[(j - oj0) * ow + (i0 - oi0)..(j - oj0) * ow + (i1 - oi0)];
                    m.data[j * w + i0..j * w + i1].copy_from_slice(row);
                    part.extend_from_slice(row);
                }
                self.renderer.light(LightUpdate { storey: k, rect: Some([i0, j0, iw, ih]), map: LightMap { origin: [0.0, 0.0], cell, w: iw, h: ih, data: part } });
            }
            _ => {
                let data = lighting::preview(&f.light, self.time, [0.0, 0.0], cell, w, h);
                let map = LightMap { origin: [0.0, 0.0], cell, w, h, data };
                f.map = Some(map.clone());
                self.renderer.light(LightUpdate { storey: k, rect: None, map });
            }
        }
    }

    fn relight_all(&mut self) {
        for f in &mut self.floors {
            f.map = None;
        }
        for k in 0..self.floors.len() {
            self.relight(k, None);
        }
    }

    /// Факел фишки: огонь в свете её этажа, свет вокруг старого и нового места — заново.
    fn torch_step(&mut self) {
        let want = self.token.filter(|_| self.torch && self.time != Time::Day).map(|t| (t.at, t.storey));
        let have = self.torch_at;
        let near = |a: [f32; 2], b: [f32; 2]| (a[0] - b[0]).hypot(a[1] - b[1]) < 0.08;
        match (have, want) {
            (None, None) => return,
            (Some((a, k)), Some((b, j))) if k == j && near(a, b) => return,
            _ => {}
        }
        let frame = |p: [f32; 2]| [p[0] - TORCH.1, p[1] - TORCH.1, p[0] + TORCH.1, p[1] + TORCH.1];
        let torch = |at: [f32; 2]| lighting::Source { at, bright: TORCH.0, dim: TORCH.1, color: [1.0, 0.86, 0.68], power: 1.0, roofed: false };
        match (have, want) {
            (Some((a, k)), Some((b, j))) if k == j => {
                if let Some(s) = self.floors[k].light.sources.last_mut() {
                    s.at = b;
                }
                let (fa, fb) = (frame(a), frame(b));
                self.relight(k, Some([fa[0].min(fb[0]), fa[1].min(fb[1]), fa[2].max(fb[2]), fa[3].max(fb[3])]));
            }
            _ => {
                if let Some((a, k)) = have {
                    self.floors[k].light.sources.pop();
                    self.relight(k, Some(frame(a)));
                }
                if let Some((b, j)) = want {
                    self.floors[j].light.sources.push(torch(b));
                    self.relight(j, Some(frame(b)));
                }
            }
        }
        self.torch_at = want;
    }

    fn door_open(&self, id: &str) -> bool {
        self.doors.get(id).is_some_and(|d| d.open)
    }

    /// Открыть или закрыть дверь: створка повернётся, проход и свет — по новому.
    fn toggle(&mut self, k: usize, i: usize) {
        let Some(d) = self.floors.get(k).and_then(|f| f.doors.get(i)) else { return };
        let (id, blocker) = (d.id.clone(), d.blocker);
        let st = self.doors.entry(id).or_default();
        st.open = !st.open;
        let open = st.open;
        if let Some(b) = self.floors[k].light.blockers.get_mut(blocker) {
            b.pass = if open { 1.0 } else { 0.0 };
        }
        // Фишка в проёме закрывающейся двери — шаг назад, в ту сторону, где стоит.
        if !open && let Some(t) = self.token.filter(|t| t.storey == k) {
            let seg = self.floors[k].doors[i].seg;
            let at = push_out(t.at, TOKEN_R, std::iter::once((seg, 0.06)));
            self.token = Some(Token { at, ..t });
        }
        // Свет — заново только там, куда достают огни у этой двери (и факел, он среди них).
        if self.time != Time::Day {
            let seg = self.floors[k].doors[i].seg;
            let area = self.floors[k]
                .light
                .sources
                .iter()
                .filter(|s| crate::map::segment_distance(s.at, seg.0, seg.1) < s.dim)
                .map(|s| [s.at[0] - s.dim, s.at[1] - s.dim, s.at[0] + s.dim, s.at[1] + s.dim])
                .reduce(|a, b| [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]);
            if let Some(area) = area {
                self.relight(k, Some(area));
            }
        }
    }

    /// Ближайшая к точке дверь этажа ближе `reach`.
    fn door_near(&self, k: usize, p: [f32; 2], reach: f32) -> Option<usize> {
        let f = self.floors.get(k)?;
        f.doors
            .iter()
            .enumerate()
            .map(|(i, d)| (i, crate::map::segment_distance(p, d.seg.0, d.seg.1)))
            .filter(|&(_, dist)| dist < reach)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    }

    /// Шаг фишки на `delta` клеток: мелкими шагами, выталкиваясь из стен и закрытых дверей.
    fn walk(&mut self, delta: [f32; 2]) {
        let Some(t) = self.token else { return };
        let Some(f) = self.floors.get(t.storey) else { return };
        let len = delta[0].hypot(delta[1]);
        if len < 1e-6 {
            return;
        }
        let steps = (len / 0.1).ceil().max(1.0) as usize;
        let half = f.half;
        let closed: Vec<Segment> = f.doors.iter().filter(|d| !self.door_open(&d.id)).map(|d| d.seg).collect();
        let segs = f.solid.iter().map(move |s| (*s, half)).chain(closed.iter().map(|s| (*s, 0.06)));
        let mut p = t.at;
        for _ in 0..steps {
            p = [p[0] + delta[0] / steps as f32, p[1] + delta[1] / steps as f32];
            p = push_out(p, TOKEN_R, segs.clone());
        }
        p = [p[0].clamp(TOKEN_R, self.size[0] - TOKEN_R), p[1].clamp(TOKEN_R, self.size[1] - TOKEN_R)];
        self.token = Some(Token { at: p, ..t });
    }

    /// Луч из точки окна на пол этажа `k`.
    fn floor_hit(&self, rect: Rect, pos: Pos2, k: usize) -> Option<[f32; 2]> {
        let eye = self.cam.eye();
        let f = render3d::norm(render3d::sub(self.cam.target, eye));
        let s = render3d::norm(render3d::cross(f, [0.0, 1.0, 0.0]));
        let u = render3d::cross(s, f);
        let (x, y) = ((pos.x - rect.center().x) / (rect.width() / 2.0), -(pos.y - rect.center().y) / (rect.height() / 2.0));
        let tan = (FOV / 2.0).tan();
        let aspect = rect.width() / rect.height().max(1.0);
        let d = [0, 1, 2].map(|i| f[i] + s[i] * x * tan * aspect + u[i] * y * tan);
        let plane = Self::floor_y(k);
        if d[1].abs() < 1e-6 {
            return None;
        }
        let t = (plane - eye[1]) / d[1];
        (t > 0.0).then(|| [eye[0] + d[0] * t, eye[2] + d[2] * t])
    }

    /// Кадр: камера, узлы видимых этажей со створками на их углах, фишка.
    fn frame(&self, rect: Rect) -> Frame {
        let eye = self.cam.eye();
        let near = (self.cam.dist * 0.02).clamp(0.05, 1.0);
        let far = (self.cam.dist + self.size[0].max(self.size[1]) * 2.0 + 50.0).max(100.0);
        let vp = render3d::mul(&render3d::perspective(FOV, rect.width() / rect.height().max(1.0), near, far), &render3d::look_at(eye, self.cam.target, [0.0, 1.0, 0.0]));
        let mut items = Vec::new();
        for (i, n) in self.nodes.iter().enumerate() {
            if n.storey > self.top || n.template {
                continue;
            }
            let yaw = n.yaw + n.door.as_ref().map_or(0.0, |(id, swing)| swing * self.doors.get(id).map_or(0.0, |d| d.t));
            let cut = if self.cut && matches!(n.part, Part::Wall | Part::Door) { Self::floor_y(n.storey) + CUT } else { f32::MAX };
            items.push(Item { node: i, model: render3d::placed(n.at, yaw), storey: n.storey, cut, caps: self.cut && n.part == Part::Wall, bias: 0.0 });
        }
        for p in self.props.iter().filter(|p| p.storey <= self.top && !(p.crown && self.cut)) {
            items.push(Item { node: p.node, model: p.model, storey: p.storey, cut: f32::MAX, caps: false, bias: p.bias });
        }
        let pawn = self.token.map(|t| (render3d::placed([t.at[0], Self::floor_y(t.storey), t.at[1]], 0.0), t.storey, [0.93, 0.74, 0.30]));
        let sky = match self.time {
            Time::Day => [0.55, 0.66, 0.78],
            Time::Dusk => [0.20, 0.20, 0.28],
            Time::Night => [0.03, 0.04, 0.07],
        };
        Frame { vp: Some(vp), eye, sun: render3d::norm([-0.45, 0.8, 0.35]), sky, items, pawn }
    }

    /// Двери докручиваются к открытому или закрытому положению. `true` — ещё в пути.
    fn swing_step(&mut self, dt: f32) -> bool {
        let mut moving = false;
        for d in self.doors.values_mut() {
            let goal = if d.open { 1.0 } else { 0.0 };
            if d.t != goal {
                let step = dt / SWING_TIME;
                d.t = if d.t < goal { (d.t + step).min(goal) } else { (d.t - step).max(goal) };
                moving = true;
            }
        }
        moving
    }

    /// Окно: панель над видом и сам вид; что попросили сделать.
    fn ui(&mut self, ui: &mut egui::Ui, stale: bool) -> Action {
        let mut action = Action::None;
        let storeys = self.floors.len();
        ui.horizontal_wrapped(|ui| {
            let before = self.time;
            ui.selectable_value(&mut self.time, Time::Day, "День");
            ui.selectable_value(&mut self.time, Time::Dusk, "Вечер");
            ui.selectable_value(&mut self.time, Time::Night, "Ночь");
            if self.time != before {
                // Факел — заново по новому времени: днём его нет.
                if let Some((_, k)) = self.torch_at.take() {
                    self.floors[k].light.sources.pop();
                }
                self.relight_all();
            }
            ui.separator();
            if storeys > 1 {
                if ui.add_enabled(self.top > 0, egui::Button::new("Ниже")).on_hover_text("Этаж ниже (PageDown)").clicked() {
                    self.change_storey(-1);
                }
                ui.label(format!("Этаж {} из {storeys}", self.top + 1));
                if ui.add_enabled(self.top + 1 < storeys, egui::Button::new("Выше")).on_hover_text("Этаж выше (PageUp)").clicked() {
                    self.change_storey(1);
                }
                ui.separator();
            }
            ui.checkbox(&mut self.cut, "Стены по пояс").on_hover_text("Срезать стены выше пояса: видно комнаты сверху");
            ui.checkbox(&mut self.torch, "Факел у фишки").on_hover_text("Свет 20 и 40 футов вокруг фишки вечером и ночью");
            if self.token.is_some() {
                ui.checkbox(&mut self.follow, "Камера за фишкой");
            }
            ui.separator();
            let label = if stale { "Снять заново •" } else { "Снять заново" };
            if ui.button(label).on_hover_text(if stale { "Карта менялась после снимка" } else { "Снять карту ещё раз" }).clicked() {
                action = Action::Recapture;
            }
        });
        let hint = "ЛКМ — повернуть, ПКМ — сдвинуть, колесо — ближе; двойной щелчок — поставить фишку; WASD — идти (Shift — бегом), E или щелчок у двери — открыть";
        let bottom = 22.0;
        let avail = ui.available_rect_before_wrap();
        let rect = Rect::from_min_max(avail.min, pos2(avail.max.x, (avail.max.y - bottom).max(avail.min.y + 50.0)));
        let resp = ui.interact(rect, id(), Sense::click_and_drag());
        ui.advance_cursor_after_rect(rect);
        if resp.clicked() || resp.drag_started() {
            resp.request_focus();
        }
        if resp.has_focus() {
            ui.memory_mut(|m| m.set_focus_lock_filter(id(), egui::EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: false }));
        }
        self.input(ui, &resp, rect);
        let painter = ui.painter_at(rect);
        self.renderer.paint(&painter, rect, self.frame(rect));
        self.painted += 1;
        self.rect = rect;
        if let Some(e) = self.renderer.error() {
            painter.text(rect.center(), egui::Align2::CENTER_CENTER, format!("3D не рисуется: {e}"), egui::FontId::proportional(15.0), Color32::from_rgb(240, 120, 100));
        }
        ui.horizontal(|ui| {
            ui.weak(format!("снято за {:.1} с", self.took));
            if !self.props.is_empty() {
                ui.separator();
                ui.weak(format!("предметов {}, моделями {}", self.props.len(), self.modelled)).on_hover_text("Модели — у предметов с полок «WoW/…» и «CC0/…»; прочие лежат плоской картинкой, как в 3D Canvas");
            }
            ui.separator();
            ui.add(egui::Label::new(egui::RichText::new(hint).weak().small()).truncate());
        });
        action
    }

    fn change_storey(&mut self, by: i32) {
        let n = self.floors.len();
        let k = (self.top as i32 + by).clamp(0, n as i32 - 1) as usize;
        self.top = k;
        if let Some(t) = &mut self.token {
            t.storey = k;
        }
        self.cam.target[1] = Self::floor_y(k) + 0.6;
    }

    fn input(&mut self, ui: &egui::Ui, resp: &egui::Response, rect: Rect) {
        let dt = ui.input(|i| i.stable_dt).min(0.05);
        let mut busy = false;
        if resp.dragged_by(PointerButton::Primary) {
            let d = resp.drag_delta();
            self.cam.yaw -= d.x * 0.008;
            self.cam.pitch = (self.cam.pitch + d.y * 0.008).clamp(0.06, 1.53);
            busy = true;
        }
        if resp.dragged_by(PointerButton::Secondary) || resp.dragged_by(PointerButton::Middle) {
            let d = resp.drag_delta();
            let (fwd, right) = self.cam.ground();
            let k = self.cam.dist * 0.0016;
            for i in 0..2 {
                self.cam.target[[0, 2][i]] += (-right[i] * d.x + fwd[i] * d.y) * k;
            }
            self.follow = false;
            busy = true;
        }
        if resp.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                self.cam.dist = (self.cam.dist * (-scroll * 0.0018).exp()).clamp(1.2, 600.0);
                busy = true;
            }
        }
        if resp.double_clicked()
            && let Some(p) = resp.interact_pointer_pos().and_then(|p| self.floor_hit(rect, p, self.top))
            && p[0] >= 0.0
            && p[1] >= 0.0
            && p[0] <= self.size[0]
            && p[1] <= self.size[1]
        {
            self.token = Some(Token { at: p, storey: self.top });
            // Поставили в стену — вытолкнуть, как при шаге.
            self.walk([0.0, 1e-4]);
            self.follow = true;
        } else if resp.clicked()
            && let Some(p) = resp.interact_pointer_pos().and_then(|p| self.floor_hit(rect, p, self.top))
            && let Some(i) = self.door_near(self.top, p, CLICK_REACH)
        {
            self.toggle(self.top, i);
        }
        if resp.has_focus() {
            let (fwd, right) = self.cam.ground();
            let (mut mx, mut my) = (0.0f32, 0.0f32);
            let (up, down, left, rt, run, door, pgup, pgdn) = ui.input(|i| {
                (
                    i.key_down(Key::W) || i.key_down(Key::ArrowUp),
                    i.key_down(Key::S) || i.key_down(Key::ArrowDown),
                    i.key_down(Key::A) || i.key_down(Key::ArrowLeft),
                    i.key_down(Key::D) || i.key_down(Key::ArrowRight),
                    i.modifiers.shift,
                    i.key_pressed(Key::E),
                    i.key_pressed(Key::PageUp),
                    i.key_pressed(Key::PageDown),
                )
            });
            for (on, v) in [(up, fwd), (down, [-fwd[0], -fwd[1]]), (left, [-right[0], -right[1]]), (rt, right)] {
                if on {
                    mx += v[0];
                    my += v[1];
                }
            }
            let l = mx.hypot(my);
            if l > 0.0 {
                let speed = SPEED * if run { 2.0 } else { 1.0 } * dt / l;
                if self.token.is_some() {
                    self.walk([mx * speed, my * speed]);
                    self.follow = true;
                } else {
                    // Без фишки клавиши водят камеру.
                    let k = speed * (self.cam.dist / 12.0).max(1.0);
                    self.cam.target[0] += mx * k;
                    self.cam.target[2] += my * k;
                }
                busy = true;
            }
            if door && let Some(t) = self.token && let Some(i) = self.door_near(t.storey, t.at, REACH) {
                self.toggle(t.storey, i);
            }
            if pgup {
                self.change_storey(1);
            }
            if pgdn {
                self.change_storey(-1);
            }
        }
        if self.follow && let Some(t) = self.token {
            let goal = [t.at[0], Self::floor_y(t.storey) + 0.6, t.at[1]];
            let k = 1.0 - (-dt * 10.0).exp();
            for (c, g) in self.cam.target.iter_mut().zip(goal) {
                *c += (g - *c) * k;
            }
            if self.cam.target.iter().zip(goal).any(|(c, g)| (g - c).abs() > 0.005) {
                busy = true;
            }
        }
        if self.swing_step(dt) {
            busy = true;
        }
        self.torch_step();
        // Клавиши и щелчки сами будят кадр; дальше — пока что-то движется.
        if busy {
            ui.ctx().request_repaint();
        }
    }
}

enum Action {
    None,
    Recapture,
}

/// Что уходит на видеокарту: узлы, их сетки, материалы, картинки — к ним добавляются образцы
/// предметов.
struct Parts {
    nodes: Vec<NodeInfo>,
    meshes: Vec<Vec<(usize, crate::scene3d::Mesh)>>,
    materials: Vec<crate::scene3d::Material>,
    images: Vec<image::RgbaImage>,
}

impl Parts {
    fn template(&mut self, prims: Vec<(usize, crate::scene3d::Mesh)>) -> usize {
        self.nodes.push(NodeInfo { part: Part::Floor, storey: 0, at: [0.0; 3], yaw: 0.0, door: None, template: true });
        self.meshes.push(prims);
        self.nodes.len() - 1
    }

    /// Модель — образцом: её картинки и материалы — в общие.
    fn add_model(&mut self, m: &crate::model3d::Model3d) -> usize {
        let base = self.images.len();
        self.images.extend(m.images.iter().cloned());
        let mut prims = Vec::new();
        for part in &m.parts {
            let mut mat = part.material.clone();
            if let crate::scene3d::Look::Picture(i) = mat.look {
                mat.look = crate::scene3d::Look::Picture(base + i);
            }
            self.materials.push(mat);
            prims.push((self.materials.len() - 1, part.mesh.clone()));
        }
        self.template(prims)
    }

    /// Плоская картинка — образцом: квадрат 1×1 на полу, середина в нуле, верх картинки — на север.
    fn add_card(&mut self, img: image::RgbaImage, tint: Option<[u8; 3]>) -> usize {
        let side = img.width().max(img.height());
        let mut img = if side > CARD_SIDE {
            let k = CARD_SIDE as f32 / side as f32;
            image::imageops::resize(&img, ((img.width() as f32 * k) as u32).max(1), ((img.height() as f32 * k) as u32).max(1), image::imageops::FilterType::Triangle)
        } else {
            img
        };
        if let Some(t) = tint {
            for p in img.pixels_mut() {
                for c in 0..3 {
                    p[c] = (p[c] as u32 * t[c] as u32 / 255) as u8;
                }
            }
        }
        self.images.push(img);
        let look = crate::scene3d::Look::Picture(self.images.len() - 1);
        self.materials.push(crate::scene3d::Material { look, alpha: crate::scene3d::Alpha::Mask, two_sided: true, repeat: false, opacity: 1.0 });
        let v = |x: f32, z: f32| crate::scene3d::Vertex { p: [x, 0.0, z], n: [0.0, 1.0, 0.0], uv: [x + 0.5, z + 0.5] };
        let mesh = crate::scene3d::Mesh { v: vec![v(-0.5, -0.5), v(0.5, -0.5), v(0.5, 0.5), v(-0.5, 0.5)], idx: vec![0, 2, 1, 0, 3, 2] };
        self.template(vec![(self.materials.len() - 1, mesh)])
    }
}

impl App {
    /// Предметы в окно 3D: у кого есть модель — моделью (образец на модель), прочие — плоской
    /// картинкой (образец на картинку и подкраску), как их кладёт 3D Canvas. Образцы — в `parts`.
    fn props_3d(&mut self, plans: &[(usize, Prop, props3d::Lift)], parts: &mut Parts) -> (Vec<PropAt>, usize) {
        let mut out = Vec::new();
        let mut modelled = 0;
        let mut models: HashMap<String, (usize, std::sync::Arc<crate::model3d::Model3d>)> = HashMap::new();
        let mut cards: HashMap<(String, Option<[u8; 3]>), Option<usize>> = HashMap::new();
        for (i, (k, p, lift)) in plans.iter().enumerate() {
            let y0 = View3d::floor_y(*k);
            let size = [p.size[0] * p.scale, p.size[1] * p.scale];
            let yaw = -p.rot.to_radians();
            let flip = if p.flip { -1.0 } else { 1.0 };
            let model = match models.get(&p.asset) {
                Some(m) => Some(m.clone()),
                None => self.prop_model(&p.asset).map(|m| {
                    let node = parts.add_model(&m);
                    models.insert(p.asset.clone(), (node, m.clone()));
                    (node, m)
                }),
            };
            if let Some((node, m)) = model {
                let s = props3d::fit(&m, size);
                out.push(PropAt { node, model: render3d::placed_scaled([p.pos[0], y0, p.pos[1]], yaw, [s[0] * flip, s[1], s[2]]), storey: *k, crown: false, bias: 0.0 });
                modelled += 1;
                continue;
            }
            let key = (p.asset.clone(), p.tint);
            let node = match cards.get(&key) {
                Some(n) => *n,
                None => {
                    let n = self.textures.rgba(&p.asset).ok().map(|img| parts.add_card(img, p.tint));
                    cards.insert(key, n);
                    n
                }
            };
            let Some(node) = node else { continue };
            // Плоские на полу — в одной плоскости чуть над ним, друг над другом по слою — сдвигом
            // глубины: разная высота давала щели между соседними плитками.
            let (h, bias) = match lift {
                props3d::Lift::Floor => (0.01, 2.0 + 2.0 * i as f32),
                props3d::Lift::Mid => (MID_Y, 0.0),
                props3d::Lift::Crown => (CROWN_Y, 0.0),
            };
            out.push(PropAt { node, model: render3d::placed_scaled([p.pos[0], y0 + h, p.pos[1]], yaw, [size[0] * flip, 1.0, size[1]]), storey: *k, crown: *lift == props3d::Lift::Crown, bias });
        }
        (out, modelled)
    }

    /// `--view3d`: открыть окно, когда карта и картинки загрузились.
    pub(super) fn view3d_auto_step(&mut self) {
        if self.view3d_auto && self.export.is_none() && self.frame >= 10 && self.pending_plan.is_none() && self.pending_osm.is_none() && !self.place.busy() && !self.library.loading() && self.textures.idle() && self.canvas.width() > 100.0 {
            self.view3d_auto = false;
            self.start_view3d();
        }
    }

    /// Окно 3D (если его ждут) открыто и нарисовало несколько кадров — можно снимать.
    pub(super) fn view3d_settled(&self) -> bool {
        !self.view3d_auto && self.view3d.as_ref().is_none_or(|v| v.painted >= 8)
    }

    /// Окно 3D глазами самопроверки.
    pub(crate) fn view3d_probe(&self) -> Option<Probe3d> {
        let v = self.view3d.as_ref()?;
        Some(Probe3d { rect: v.rect, yaw: v.cam.yaw, token: v.token.map(|t| t.at), open: v.doors.values().filter(|d| d.open).count(), painted: v.painted, error: v.renderer.error(), props: v.props.len(), modelled: v.modelled })
    }

    /// Снять карту для окна 3D: как выгрузка, но только пол без стен, без файлов.
    pub(crate) fn start_view3d(&mut self) {
        let side = self.map.width.max(self.map.height).max(1) as f32;
        let px = ((FLOOR_SIDE / side).floor() as u32).clamp(8, 96);
        let started = crate::plat::Instant::now();
        self.start_export(PathBuf::new(), String::new(), px, false);
        if let Some(e) = &mut self.export {
            e.view3d = true;
            e.canopies = true;
            e.three_d = false;
            e.began = started;
        }
    }

    /// Снимок готов: собрать сцену, свет и открыть окно (у открытого — камера и фишка те же).
    pub(super) fn finish_view3d(&mut self, e: &Export) {
        let mut layouts = Vec::new();
        let mut floors = Vec::new();
        let mut pics: HashMap<String, image::RgbaImage> = HashMap::new();
        for (k, img) in e.done.iter().enumerate() {
            let map = self.export_level(e, k);
            layouts.push(export::layout(map, &map.room_edges()));
            let holes = e.whites.get(k).and_then(|w| w.as_ref()).and_then(|white| export::floor_from_pair(img, white));
            let alpha = holes.is_some();
            let rgba = holes.unwrap_or_else(|| image::DynamicImage::ImageRgb8(img.clone()).to_rgba8());
            let key = format!("floor_{}", k + 1);
            floors.push(Some(scene3d::Picture { key: key.clone(), size: [rgba.width(), rgba.height()], alpha }));
            pics.insert(key, rgba);
        }
        let storeys = self.storeys_3d(e, &layouts, floors, &mut pics);
        let map0 = self.export_level(e, 0);
        let size = [map0.width as f32, map0.height as f32];
        let first_time = time_of(map0);
        // Предметы, что не в картинке пола (они в выгрузке своими тайлами), — по этажам, снизу вверх по слоям.
        let mut plans: Vec<(usize, Prop, props3d::Lift)> = Vec::new();
        for k in 0..e.done.len() {
            let m = self.export_level(e, k);
            let mut here: Vec<(usize, Prop, props3d::Lift)> = m.props.iter().filter_map(|p| props3d::lift(p, m).map(|l| (k, p.clone(), l))).collect();
            here.sort_by_key(|(_, p, _)| p.layer());
            plans.extend(here);
        }
        let mut scene = scene3d::build(size, &storeys);
        let mut images = Vec::new();
        for p in &scene.pictures {
            match pics.remove(&p.key) {
                Some(img) => images.push(img),
                None => {
                    self.error = Some(format!("3D: нет картинки {}", p.key));
                    return;
                }
            }
        }
        // Картинки шире, чем потянет видеокарта, — мельче (у огромной карты пол).
        for img in &mut images {
            let side = img.width().max(img.height());
            if side as f32 > FLOOR_SIDE {
                let k = FLOOR_SIDE / side as f32;
                *img = image::imageops::resize(img, ((img.width() as f32 * k) as u32).max(1), ((img.height() as f32 * k) as u32).max(1), image::imageops::FilterType::Triangle);
            }
        }
        let nodes = scene
            .nodes
            .iter()
            .map(|n| NodeInfo {
                part: n.part,
                storey: n.storey,
                at: n.at,
                yaw: n.yaw,
                door: n.door.as_ref().map(|d| (d.id.trim_end_matches("_a").trim_end_matches("_b").to_owned(), d.swing)),
                template: false,
            })
            .collect();
        let meshes = scene.nodes.iter_mut().map(|n| std::mem::take(&mut n.prims)).collect();
        let mut parts = Parts { nodes, meshes, materials: std::mem::take(&mut scene.materials), images };
        let (props, modelled) = self.props_3d(&plans, &mut parts);
        let Parts { nodes, meshes, materials, images } = parts;
        let floors3: Vec<Floor3> = layouts.iter().enumerate().map(|(k, lay)| Floor3::new(k, lay, self.export_level(e, k).rooms.clone(), storeys[k].look.width.max(0.02) / 2.0, e.px as f32)).collect();
        let renderer = self.view3d.as_ref().map_or_else(Renderer::default, |v| v.renderer.clone());
        renderer.upload(Upload { nodes: meshes, materials, images, pawn: render3d::pawn() });
        let mut v = View3d::fresh(renderer, nodes, floors3, size, first_time);
        v.props = props;
        v.modelled = modelled;
        v.rev = self.rev;
        v.took = e.began.elapsed().as_secs_f32();
        // Пересняли — камера, время, фишка и открытые двери те же.
        if let Some(old) = self.view3d.take() {
            v.carry(old);
        }
        if let Ok(preset) = std::env::var("KARTOGRAF_VIEW3D") {
            v.preset(&preset);
        }
        v.relight_all();
        self.view3d = Some(v);
    }

    /// Окно 3D, если открыто; закрытое отдаёт видеокарте память.
    pub(super) fn view3d_window(&mut self, ctx: &egui::Context) {
        let Some(v) = &mut self.view3d else { return };
        let stale = v.rev != self.rev;
        let mut open = v.open;
        let mut action = Action::None;
        egui::Window::new("Тестовая зона 3D").open(&mut open).pivot(egui::Align2::CENTER_CENTER).default_pos(ctx.content_rect().center()).default_size([960.0, 640.0]).min_size([420.0, 300.0]).resizable(true).collapsible(false).show(ctx, |ui| {
            action = v.ui(ui, stale);
        });
        if !open {
            let renderer = v.renderer.clone();
            self.view3d = None;
            renderer.release(&ctx.layer_painter(egui::LayerId::background()));
            return;
        }
        if let Action::Recapture = action {
            self.start_view3d();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::export::{Layout, Light, PortalOut};

    /// Комната 6×4 клетки со стенами по краю и дверью посреди южной стены, лампа внутри.
    fn room() -> View3d {
        let lay = Layout {
            tents: Vec::new(),
            walls: vec![vec![[5.5, 6.0], [8.0, 6.0], [8.0, 2.0], [2.0, 2.0], [2.0, 6.0], [4.5, 6.0]]],
            low: Vec::new(),
            cliffs: Vec::new(),
            portals: vec![PortalOut { seg: ([4.5, 6.0], [5.5, 6.0]), kind: PortalKind::Door, leaf: None }],
            lights: vec![Light { at: [5.0, 4.0], bright: 2.0, dim: 4.0, color: "#ffffff".into(), animation: String::new(), look: (0.5, 1, 0.0, 0.0), tempo: (5, 5) }],
            zones: Vec::new(),
            passages: Vec::new(),
        };
        let floor = Floor3::new(0, &lay, Mask::for_cells(10, 10), 0.1, 100.0);
        let mut v = View3d::fresh(Renderer::default(), Vec::new(), vec![floor], [10.0, 10.0], Time::Night);
        v.relight_all();
        v
    }

    fn walk_to(v: &mut View3d, from: [f32; 2], to: [f32; 2]) -> [f32; 2] {
        v.token = Some(Token { at: from, storey: 0 });
        let d = [(to[0] - from[0]) / 60.0, (to[1] - from[1]) / 60.0];
        for _ in 0..60 {
            v.walk(d);
        }
        v.token.map(|t| t.at).unwrap_or_default()
    }

    /// Светлота карты света в точке (среднее множителя по каналам).
    fn light_at(v: &View3d, p: [f32; 2]) -> f32 {
        let m = v.floors[0].map.as_ref().expect("карта света");
        let (i, j) = (((p[0] - m.origin[0]) / m.cell) as usize, ((p[1] - m.origin[1]) / m.cell) as usize);
        let t = m.data[j * m.w + i];
        (t[0] + t[1] + t[2]) / 3.0
    }

    #[test]
    fn the_door_id_matches_the_leaf_of_the_scene() {
        let v = room();
        assert_eq!(v.floors[0].doors[0].id, "0:450,600,550,600");
        // Преграда света этой двери — сразу за пятью отрезками стены.
        assert_eq!(v.floors[0].doors[0].blocker, 5);
        assert_eq!(v.floors[0].light.blockers[5].a, [4.5, 6.0]);
    }

    #[test]
    fn a_token_stops_at_a_wall_and_slides_along_it() {
        let mut v = room();
        let at = walk_to(&mut v, [5.0, 4.0], [5.0, 0.5]);
        assert!(at[1] >= 2.0 + 0.1 + TOKEN_R - 1e-3, "в стену не вошла: {at:?}");
        // Наискось в стену — скользит вдоль неё.
        let at = walk_to(&mut v, [4.0, 3.0], [7.0, 1.0]);
        assert!(at[0] > 6.0 && at[1] >= 2.0 + 0.1 + TOKEN_R - 1e-3, "{at:?}");
    }

    #[test]
    fn a_closed_door_holds_the_token_and_an_open_one_lets_it_out() {
        let mut v = room();
        let at = walk_to(&mut v, [5.0, 4.0], [5.0, 8.0]);
        assert!(at[1] < 6.0, "закрытая дверь держит: {at:?}");
        v.toggle(0, 0);
        assert!(v.door_open("0:450,600,550,600"));
        let at = walk_to(&mut v, [5.0, 4.0], [5.0, 8.0]);
        assert!(at[1] > 7.5, "в открытую вышла: {at:?}");
    }

    #[test]
    fn closing_the_door_on_the_token_pushes_it_out_of_the_doorway() {
        let mut v = room();
        v.toggle(0, 0);
        v.token = Some(Token { at: [5.0, 6.05], storey: 0 });
        v.toggle(0, 0);
        let at = v.token.map(|t| t.at).unwrap_or_default();
        assert!((at[1] - 6.0).abs() >= TOKEN_R, "{at:?}");
    }

    #[test]
    fn an_open_door_lets_the_lamp_light_out() {
        let mut v = room();
        let outside = [5.0, 7.0];
        let shut = light_at(&v, outside);
        v.toggle(0, 0);
        let open = light_at(&v, outside);
        assert!(open > shut + 0.1, "за открытой дверью светлее: {shut} → {open}");
        v.toggle(0, 0);
        assert!((light_at(&v, outside) - shut).abs() < 1e-4, "закрыли — как было");
    }

    #[test]
    fn the_torch_lights_around_the_token_and_goes_out_with_it() {
        let mut v = room();
        let spot = [5.0, 9.0];
        let dark = light_at(&v, spot);
        v.token = Some(Token { at: [5.0, 8.5], storey: 0 });
        v.torch_step();
        let lit = light_at(&v, spot);
        assert!(lit > dark + 0.1, "факел светит: {dark} → {lit}");
        v.token = None;
        v.torch_step();
        assert!((light_at(&v, spot) - dark).abs() < 1e-4, "фишку убрали — темно");
        assert_eq!(v.floors[0].light.sources.len(), 1, "свой огонь фишки убран из света этажа");
    }

    #[test]
    fn a_click_on_the_floor_lands_where_the_camera_looks() {
        let mut v = room();
        v.cam = Orbit { target: [5.0, 0.0, 4.0], yaw: 0.7, pitch: 0.9, dist: 12.0 };
        let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 500.0));
        let p = v.floor_hit(rect, rect.center(), 0).expect("пол");
        assert!((p[0] - 5.0).abs() < 1e-3 && (p[1] - 4.0).abs() < 1e-3, "{p:?}");
        // Правее середины окна — правее по взгляду камеры.
        let r = v.floor_hit(rect, rect.center() + vec2(100.0, 0.0), 0).expect("пол");
        let (_, right) = v.cam.ground();
        assert!((r[0] - p[0]) * right[0] + (r[1] - p[1]) * right[1] > 0.5);
    }
}
