//! Тестовая зона: окно 3D в Картографе. Карта снимается так же, как для `.glb` в выгрузку
//! (пол без стен на каждый этаж), из снимка строится та же сцена [`crate::scene3d`], и окно
//! её рисует ([`crate::render3d`]): облёт камерой, фишка ходит со столкновениями о стены и
//! закрытые двери, двери открываются, свет огней — как на ночном снимке и в Foundry.
//!
//! Окно — снимок карты, но предметы, что стоят в 3D своими моделями и картинками, живые: их
//! правка на карте (и их огни) видна сразу. Прочие правки — после «Снять заново».
//!
//! Правка в 3D (Tab): щелчок выбирает предмет — тот же выбор, что на карте; протяжка
//! переносит по полу, Q и E поворачивают, Alt с колесом — размер, Delete убирает; с полки
//! инструмента «Объект» щелчок по полу ставит новый. Правку делает карта ([`App::apply_3d`]) —
//! с отменой, как правку мышью на холсте.

use super::*;
use crate::lighting::{self, Time};
use crate::render3d::{self, Frame, Item, LightMap, LightUpdate, Renderer, Upload};
use crate::scene3d::{self, Part, STOREY};
use super::props3d;
use std::collections::HashMap;
use std::sync::Arc;

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
    /// Крыша: её контур на карте — под ним фишка, и крыша прячется, как тайл крыши в Foundry.
    roof: Vec<Vec<[f32; 2]>>,
}

/// Предмет на месте: образец, матрица, этаж; крона прячется, когда стены по пояс; `bias` —
/// порядок плоской картинки на полу (сдвиг глубины). Для выбора лучом: номер на карте этажа,
/// где стоит, поворот и растяжка образца, его рамка.
struct PropAt {
    node: usize,
    model: render3d::Mat4,
    storey: usize,
    crown: bool,
    bias: f32,
    index: usize,
    at: [f32; 3],
    yaw: f32,
    scale: [f32; 3],
    bounds: [[f32; 3]; 2],
}

impl PropAt {
    /// Где луч из `eye` по `dir` входит в рамку предмета; мимо — нет.
    fn hit(&self, eye: [f32; 3], dir: [f32; 3]) -> Option<f32> {
        // В оси образца: сдвиг, поворот назад, растяжка.
        let (s, c) = self.yaw.sin_cos();
        let local = |d: [f32; 3]| [(c * d[0] - s * d[2]) / self.scale[0], d[1] / self.scale[1], (s * d[0] + c * d[2]) / self.scale[2]];
        let (o, d) = (local(render3d::sub(eye, self.at)), local(dir));
        let [lo, hi] = self.bounds;
        let (mut t0, mut t1) = (0.0f32, f32::MAX);
        for a in 0..3 {
            if d[a].abs() < 1e-9 {
                if o[a] < lo[a] || o[a] > hi[a] {
                    return None;
                }
                continue;
            }
            let (ta, tb) = ((lo[a] - o[a]) / d[a], (hi[a] - o[a]) / d[a]);
            t0 = t0.max(ta.min(tb));
            t1 = t1.min(ta.max(tb));
            if t0 > t1 {
                return None;
            }
        }
        Some(t0)
    }

    /// Рёбра рамки в мире.
    fn edges(&self) -> Vec<([f32; 3], [f32; 3])> {
        let [lo, hi] = self.bounds;
        let corner = |k: usize| render3d::apply(&self.model, [if k & 1 == 0 { lo[0] } else { hi[0] }, if k & 2 == 0 { lo[1] } else { hi[1] }, if k & 4 == 0 { lo[2] } else { hi[2] }]);
        let mut out = Vec::new();
        for k in 0..8usize {
            for bit in [1, 2, 4] {
                if k & bit == 0 {
                    out.push((corner(k), corner(k | bit)));
                }
            }
        }
        out
    }
}

/// Образцы предметов, уже отданные видеокарте: модель — по картинке (нет модели — `None`),
/// плоская картинка — по картинке и подкраске (не прочиталась — `None`).
#[derive(Default)]
struct Templates {
    models: HashMap<String, Option<(usize, Arc<crate::model3d::Model3d>)>>,
    cards: HashMap<(String, Option<[u8; 3]>), Option<usize>>,
}

/// Правка из окна 3D: этаж и номер предмета на нём. Делает её карта после окна
/// ([`App::apply_3d`]).
#[derive(Clone, Debug, PartialEq)]
enum Edit3 {
    /// Выбрать предмет или снять выбор.
    Select(Option<(usize, usize)>),
    /// Точка отмены перед правкой этажа.
    Begin(usize),
    Move(usize, usize, [f32; 2]),
    /// Повернуть на столько градусов по часовой (вид сверху, как на карте).
    Turn(usize, usize, f32),
    /// Растянуть во столько раз.
    Scale(usize, usize, f32),
    Delete(usize, usize),
    /// Поставить, что выбрано у «Объекта», в точку этажа; `alt` — привязка наоборот.
    Place(usize, [f32; 2], bool),
}

/// Что поставит щелчок в правке: рамка предмета с полки (клетки, с размером) и поворот.
#[derive(Clone, Copy, PartialEq)]
struct Ghost {
    size: [f32; 2],
    rot: f32,
}

/// Предмет, который тащат: этаж и номер на карте, высота плоскости протяжки и где взяли —
/// от середины предмета.
#[derive(Clone, Copy)]
struct Grab {
    storey: usize,
    index: usize,
    plane: f32,
    offset: [f32; 2],
}

/// Выбранное на карте: этаж, номер предмета и его рамка на полу.
#[derive(Clone, Copy, PartialEq)]
struct Picked {
    storey: usize,
    index: usize,
    corners: [[f32; 2]; 4],
}

/// Шаг поворота в правке, градусы: Q и E — крупный, с Shift — мелкий.
const TURN_STEP: (f32, f32) = (15.0, 1.0);

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
    /// Земля этажа: рельеф, помосты, ступени, марши к этажу выше.
    ground: scene3d::Ground,
    /// Где у этажа пол (у первого — везде): над дырой фишка сходит на этаж ниже.
    cover: Option<scene3d::Cover>,
    /// Что закрывает предметы от взгляда при выборе: отрезок стены и её высота над полом.
    occluders: Vec<(Segment, f32)>,
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
    /// Ступни над полом этажа: на помосте, ступени, склоне.
    feet: f32,
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
    /// Крыши видны (над фишкой — всё равно прячутся).
    roofs: bool,
    /// Камера идёт за фишкой.
    follow: bool,
    /// Где сейчас горит факел фишки: место и этаж.
    torch_at: Option<([f32; 2], usize)>,
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
    /// Образцы предметов на видеокарте: новый предмет на карте добавляет свой.
    templates: Templates,
    /// Предметы каждого этажа, по которым поставлены `props`: карта их поменяла — переставить.
    seen: Vec<Vec<Prop>>,
    /// Лестницы, ставшие маршами при снимке (картинка и место) — в `props` их нет.
    flights: Vec<Vec<(String, [f32; 2])>>,
    /// Этажи при снимке без живых предметов: что-то из этого поменялось — «Снять заново».
    base: Vec<Map>,
    /// Карта менялась не только живыми предметами; с какой правкой (`App::rev`) сверено.
    stale: bool,
    checked: u64,
    /// Правка (Tab): мышь выбирает и тащит предметы, а не только камеру.
    edit: bool,
    grab: Option<Grab>,
    /// Предмет, который вертит или растягивает колесо: правка в истории на него одна.
    wheel: Option<(usize, usize)>,
    /// Предмет под курсором в правке — номер в `props`.
    hover: Option<usize>,
    /// Выбранное на карте, что ставит щелчок, привязка к сетке — от карты, на кадр.
    picked: Option<Picked>,
    ghost: Option<Ghost>,
    snap: bool,
    /// Правки этого кадра — карте.
    edits: Vec<Edit3>,
    /// Вид прошлого кадра — для выбора и самопроверки.
    vp: Option<render3d::Mat4>,
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
    /// Этаж фишки и высота её ступней над его полом.
    pub storey: usize,
    pub feet: f32,
    /// Крыш в сцене.
    pub roofs: usize,
    /// Окно в правке; карта менялась не только живыми предметами (пора снять заново).
    pub edit: bool,
    pub stale: bool,
    /// Где на экране середины предметов: этаж, номер на карте, точка.
    pub spots: Vec<(usize, usize, Pos2)>,
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
    fn new(k: usize, lay: &export::Layout, rooms: Mask, half: f32, px: f32, ground: scene3d::Ground, cover: Option<scene3d::Cover>) -> Floor3 {
        let light = lighting::Scene::new(lay, rooms, Vec::new());
        // Преграды света — сначала отрезки стен, потом двери и окна по порядку.
        let walls_n: usize = lay.walls.iter().map(|w| w.len().saturating_sub(1)).sum();
        let at = |v: [f32; 2]| [(v[0] * px).round() as i64, (v[1] * px).round() as i64];
        let mut solid: Vec<Segment> = segments(&lay.walls).chain(segments(&lay.low)).chain(segments(&lay.cliffs)).collect();
        let mut doors = Vec::new();
        // Стены во всю высоту, низкие и подоконники — по пояс (сквозь стекло видно).
        let mut occluders: Vec<(Segment, f32)> = segments(&lay.walls).map(|s| (s, STOREY)).chain(segments(&lay.low).map(|s| (s, scene3d::LOW))).collect();
        for (i, po) in lay.portals.iter().enumerate() {
            if po.kind.is_door() || po.kind == PortalKind::Secret {
                let (a, b) = (at(po.seg.0), at(po.seg.1));
                doors.push(Door3 { seg: po.seg, id: format!("{k}:{},{},{},{}", a[0], a[1], b[0], b[1]), blocker: walls_n + i });
            } else {
                solid.push(po.seg);
                if po.kind == PortalKind::Window {
                    occluders.push((po.seg, scene3d::SILL));
                }
            }
        }
        Floor3 { half, solid, doors, light, map: None, ground, cover, occluders }
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
            roofs: true,
            follow: false,
            torch_at: None,
            took: 0.0,
            painted: 0,
            rect: Rect::NOTHING,
            props: Vec::new(),
            modelled: 0,
            templates: Templates::default(),
            seen: Vec::new(),
            flights: Vec::new(),
            base: Vec::new(),
            stale: false,
            checked: 0,
            edit: false,
            grab: None,
            wheel: None,
            hover: None,
            picked: None,
            ghost: None,
            snap: false,
            edits: Vec::new(),
            vp: None,
        }
    }

    /// Взять у прежнего окна камеру, время, вид, фишку и открытые двери, что остались на месте.
    /// Карта другого размера — другая карта: камера, фишка и двери — с нуля.
    fn carry(&mut self, old: View3d) {
        let n = self.floors.len();
        self.cut = old.cut;
        self.torch = old.torch;
        self.roofs = old.roofs;
        self.edit = old.edit;
        if old.size != self.size {
            return;
        }
        self.time = old.time;
        self.cam = old.cam;
        self.top = old.top.min(n.saturating_sub(1));
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
                "noroofs" => self.roofs = false,
                "edit" => self.edit = true,
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
                        self.place_token([v[0], v[1]], storey);
                        if let Some(t) = self.token {
                            self.cam.target = [t.at[0], Self::floor_y(t.storey) + t.feet + 0.6, t.at[1]];
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

    /// Огни предметов этажа `k` по карте (`lights` — [`export::prop_lights`]): какие сдвинулись,
    /// появились или пропали — вокруг них свет заново. Факел фишки остаётся последним.
    fn prop_lights(&mut self, k: usize, lights: &[export::Light]) {
        let torch = self.torch_at.is_some_and(|(_, j)| j == k);
        let Some(f) = self.floors.get_mut(k) else { return };
        let want: Vec<lighting::Source> = lights.iter().map(|l| lighting::Source::of(l, &[])).collect();
        let n = f.light.sources.len().saturating_sub(torch as usize);
        if f.light.sources[..n] == want[..] {
            return;
        }
        let frame = |s: &lighting::Source| [s.at[0] - s.dim, s.at[1] - s.dim, s.at[0] + s.dim, s.at[1] + s.dim];
        let mut area: Option<[f32; 4]> = None;
        for i in 0..n.max(want.len()) {
            let (a, b) = (f.light.sources[..n].get(i), want.get(i));
            if a != b {
                for r in [a, b].into_iter().flatten().map(frame) {
                    area = Some(area.map_or(r, |q| [q[0].min(r[0]), q[1].min(r[1]), q[2].max(r[2]), q[3].max(r[3])]));
                }
            }
        }
        let fire = torch.then(|| f.light.sources.last().cloned()).flatten();
        f.light.sources = want;
        f.light.sources.extend(fire);
        if let Some(a) = area
            && self.time != Time::Day
        {
            self.relight(k, Some(a));
        }
    }

    /// Поставить фишку на этаж `k` в точку `p` — на верх того, что там стоит (помост, ступень),
    /// и вытолкнуть из стен, как при шаге.
    fn place_token(&mut self, p: [f32; 2], k: usize) {
        let feet = self.floors.get(k).map_or(0.0, |f| f.ground.at(p));
        self.token = Some(Token { at: p, storey: k, feet });
        self.top = k;
        self.walk([0.0, 1e-4]);
        self.follow = true;
    }

    /// Куда встанет фишка, шагнув на этаже `k` в `p` со ступнями на `feet`: этаж и высота
    /// ступней на нём; `None` — не пройти (стенка помоста, обрыв, дыра в полу глубже шага).
    /// Марш довёл до пола этажа выше — она уже там; над дырой в полу сходит на марш этажа ниже.
    fn footing(&self, k: usize, p: [f32; 2], feet: f32) -> Option<(usize, f32)> {
        let f = self.floors.get(k)?;
        let g = f.ground.step(p, feet)?;
        if k > 0 && g < 1e-3 && f.cover.as_ref().is_some_and(|c| !c.has(p)) {
            // Пола нет — под ногами этаж ниже: его марш (верх — у пола этого этажа) или пустота.
            let below = self.floors[k - 1].ground.step(p, feet + STOREY)?;
            if below >= STOREY - 0.05 {
                return Some((k, 0.0));
            }
            return (below - STOREY >= feet - 1.5 * scene3d::STEP).then_some((k - 1, below));
        }
        if g >= STOREY - 0.05 && k + 1 < self.floors.len() {
            return Some((k + 1, g - STOREY));
        }
        Some((k, g))
    }

    /// Шаг фишки на `delta` клеток: мелкими шагами, выталкиваясь из стен и закрытых дверей; по
    /// земле — вверх не выше шага ([`scene3d::STEP`]), вниз — сходит; не пройти — скользит вдоль.
    fn walk(&mut self, delta: [f32; 2]) {
        let Some(t) = self.token else { return };
        let len = delta[0].hypot(delta[1]);
        if len < 1e-6 {
            return;
        }
        let steps = (len / 0.1).ceil().max(1.0) as usize;
        let (mut p, mut k, mut feet) = (t.at, t.storey, t.feet);
        let d = [delta[0] / steps as f32, delta[1] / steps as f32];
        for _ in 0..steps {
            let Some(f) = self.floors.get(k) else { break };
            let half = f.half;
            let closed: Vec<Segment> = f.doors.iter().filter(|d| !self.door_open(&d.id)).map(|d| d.seg).collect();
            let segs = f.solid.iter().map(move |s| (*s, half)).chain(closed.iter().map(|s| (*s, 0.06)));
            // Прямо, а упёрлись в стенку помоста — вдоль неё по одной оси.
            for try_d in [d, [d[0], 0.0], [0.0, d[1]]] {
                if try_d == [0.0, 0.0] {
                    continue;
                }
                let q = push_out([p[0] + try_d[0], p[1] + try_d[1]], TOKEN_R, segs.clone());
                let q = [q[0].clamp(TOKEN_R, self.size[0] - TOKEN_R), q[1].clamp(TOKEN_R, self.size[1] - TOKEN_R)];
                if let Some((k2, f2)) = self.footing(k, q, feet) {
                    (p, k, feet) = (q, k2, f2);
                    break;
                }
            }
        }
        if k != t.storey {
            self.top = k;
        }
        self.token = Some(Token { at: p, storey: k, feet });
    }

    /// Луч из точки окна: глаз и направление.
    fn ray(&self, rect: Rect, pos: Pos2) -> ([f32; 3], [f32; 3]) {
        let eye = self.cam.eye();
        let f = render3d::norm(render3d::sub(self.cam.target, eye));
        let s = render3d::norm(render3d::cross(f, [0.0, 1.0, 0.0]));
        let u = render3d::cross(s, f);
        let (x, y) = ((pos.x - rect.center().x) / (rect.width() / 2.0), -(pos.y - rect.center().y) / (rect.height() / 2.0));
        let tan = (FOV / 2.0).tan();
        let aspect = rect.width() / rect.height().max(1.0);
        (eye, [0, 1, 2].map(|i| f[i] + s[i] * x * tan * aspect + u[i] * y * tan))
    }

    /// Луч из точки окна на плоскость высотой `y`.
    fn plane_hit(&self, rect: Rect, pos: Pos2, y: f32) -> Option<[f32; 2]> {
        let (eye, d) = self.ray(rect, pos);
        if d[1].abs() < 1e-6 {
            return None;
        }
        let t = (y - eye[1]) / d[1];
        (t > 0.0).then(|| [eye[0] + d[0] * t, eye[2] + d[2] * t])
    }

    /// Луч из точки окна на пол этажа `k`.
    fn floor_hit(&self, rect: Rect, pos: Pos2, k: usize) -> Option<[f32; 2]> {
        self.plane_hit(rect, pos, Self::floor_y(k))
    }

    /// Предмет виден в кадре: его этаж показан, крона — не при стенах по пояс.
    fn shown(&self, p: &PropAt) -> bool {
        p.storey <= self.top && !(p.crown && self.cut)
    }

    /// Крыша видна в кадре (над фишкой и при стенах по пояс — спрятана).
    fn roof_shown(&self, n: &NodeInfo) -> bool {
        n.part == Part::Roof && n.storey <= self.top && self.roofs && !self.cut && !self.token.is_some_and(|t| t.storey <= n.storey && scene3d::inside(&n.roof, t.at))
    }

    /// Как далеко по лучу первое, что закрывает предметы: стена, пол этажа выше, видная крыша.
    fn blocked(&self, eye: [f32; 3], d: [f32; 3]) -> f32 {
        let mut best = f32::MAX;
        let plane = |y: f32| (d[1].abs() > 1e-6).then(|| (y - eye[1]) / d[1]).filter(|t| *t > 0.0);
        for (k, f) in self.floors.iter().enumerate().take(self.top + 1) {
            let y0 = Self::floor_y(k);
            if k > 0
                && let Some(t) = plane(y0)
                && t < best
                && f.cover.as_ref().is_none_or(|c| c.has([eye[0] + d[0] * t, eye[2] + d[2] * t]))
            {
                best = t;
            }
            for &((a, b), h) in &f.occluders {
                let h = if self.cut { h.min(CUT) } else { h };
                let e = [b[0] - a[0], b[1] - a[1]];
                let den = d[0] * e[1] - d[2] * e[0];
                if den.abs() < 1e-9 {
                    continue;
                }
                let w = [a[0] - eye[0], a[1] - eye[2]];
                let t = (w[0] * e[1] - w[1] * e[0]) / den;
                let u = (w[0] * d[2] - w[1] * d[0]) / den;
                let y = eye[1] + d[1] * t;
                if t > 0.0 && t < best && (0.0..=1.0).contains(&u) && y >= y0 && y <= y0 + h {
                    best = t;
                }
            }
        }
        // Крыша — по плоскости свеса: низ её скатов, предметы в доме — под ней.
        for n in self.nodes.iter().filter(|n| self.roof_shown(n)) {
            if let Some(t) = plane(Self::floor_y(n.storey) + STOREY)
                && t < best
                && scene3d::inside(&n.roof, [eye[0] + d[0] * t, eye[2] + d[2] * t])
            {
                best = t;
            }
        }
        best
    }

    /// Предмет под точкой окна — номер в `props`: ближний по лучу, не за стеной, полом или крышей.
    fn pick(&self, rect: Rect, pos: Pos2) -> Option<usize> {
        let (eye, d) = self.ray(rect, pos);
        let wall = self.blocked(eye, d);
        self.props.iter().enumerate().filter(|(_, p)| self.shown(p)).filter_map(|(i, p)| p.hit(eye, d).filter(|t| *t < wall).map(|t| (i, t))).min_by(|a, b| a.1.total_cmp(&b.1)).map(|(i, _)| i)
    }

    /// Предмет карты (этаж, номер) в `props`.
    fn prop_of(&self, storey: usize, index: usize) -> Option<&PropAt> {
        self.props.iter().find(|p| p.storey == storey && p.index == index)
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
            // Крыша над фишкой прячется, как тайл крыши в Foundry; со стенами по пояс — все.
            if n.part == Part::Roof && !self.roof_shown(n) {
                continue;
            }
            let yaw = n.yaw + n.door.as_ref().map_or(0.0, |(id, swing)| swing * self.doors.get(id).map_or(0.0, |d| d.t));
            let cut = if self.cut && matches!(n.part, Part::Wall | Part::Door) { Self::floor_y(n.storey) + CUT } else { f32::MAX };
            items.push(Item { node: i, model: render3d::placed(n.at, yaw), storey: n.storey, cut, caps: self.cut && n.part == Part::Wall, bias: 0.0 });
        }
        for p in self.props.iter().filter(|p| self.shown(p)) {
            items.push(Item { node: p.node, model: p.model, storey: p.storey, cut: f32::MAX, caps: false, bias: p.bias });
        }
        let pawn = self.token.map(|t| (render3d::placed([t.at[0], Self::floor_y(t.storey) + t.feet, t.at[1]], 0.0), t.storey, [0.93, 0.74, 0.30]));
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
    fn ui(&mut self, ui: &mut egui::Ui) -> Action {
        let mut action = Action::None;
        let storeys = self.floors.len();
        let stale = self.stale;
        ui.horizontal_wrapped(|ui| {
            if ui.selectable_label(self.edit, "Правка").on_hover_text("Выбирать, переносить, вертеть и ставить предметы прямо в 3D (Tab)").clicked() {
                self.set_edit(!self.edit);
            }
            ui.separator();
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
            ui.checkbox(&mut self.roofs, "Крыши").on_hover_text("Крыши домов; над фишкой крыша прячется сама, как в Foundry");
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
        let hint = if self.edit {
            "Правка: щелчок — выбрать предмет, протяжка — перенести (Alt — без привязки), Q/E — повернуть (Shift — на градус), Alt+колесо — размер, Delete — убрать; с полки «Объекта» щелчок по полу ставит; Tab — к фишке"
        } else {
            "ЛКМ — повернуть, ПКМ — сдвинуть, колесо — ближе; двойной щелчок — поставить фишку; WASD — идти (Shift — бегом), E или щелчок у двери — открыть; Tab — правка"
        };
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
        let frame = self.frame(rect);
        self.vp = frame.vp;
        self.renderer.paint(&painter, rect, frame);
        self.painted += 1;
        self.rect = rect;
        self.overlay(&painter, rect, resp.hover_pos());
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
            t.feet = self.floors[k].ground.at(t.at);
        }
        self.cam.target[1] = Self::floor_y(k) + 0.6;
    }

    fn input(&mut self, ui: &egui::Ui, resp: &egui::Response, rect: Rect) {
        let dt = ui.input(|i| i.stable_dt).min(0.05);
        let mut busy = false;
        let (alt, shift) = ui.input(|i| (i.modifiers.alt, i.modifiers.shift));
        if resp.has_focus() && ui.input(|i| i.key_pressed(Key::Tab)) {
            self.set_edit(!self.edit);
        }
        if ui.input(|i| i.pointer.any_pressed()) {
            self.wheel = None;
        }
        self.hover = if self.edit && self.grab.is_none() { resp.hover_pos().and_then(|p| self.pick(rect, p)) } else { None };
        // Правка: протяжка с предмета тащит его по полу, мимо — вертит камеру.
        if self.edit
            && resp.drag_started_by(PointerButton::Primary)
            && let Some(o) = ui.input(|i| i.pointer.press_origin())
            && let Some(j) = self.pick(rect, o)
        {
            let p = &self.props[j];
            let (k, i, plane, at) = (p.storey, p.index, p.at[1], p.at);
            if let Some(w) = self.plane_hit(rect, o, plane) {
                self.grab = Some(Grab { storey: k, index: i, plane, offset: [w[0] - at[0], w[1] - at[2]] });
                self.edits.push(Edit3::Select(Some((k, i))));
                self.edits.push(Edit3::Begin(k));
            }
        }
        if let Some(g) = self.grab {
            if resp.dragged_by(PointerButton::Primary)
                && let Some(w) = resp.interact_pointer_pos().and_then(|p| self.plane_hit(rect, p, g.plane))
            {
                let pos = self.snapped([w[0] - g.offset[0], w[1] - g.offset[1]], alt);
                let pos = [pos[0].clamp(0.0, self.size[0]), pos[1].clamp(0.0, self.size[1])];
                self.edits.push(Edit3::Move(g.storey, g.index, pos));
                busy = true;
            }
        } else if resp.dragged_by(PointerButton::Primary) {
            let d = resp.drag_delta();
            self.cam.yaw -= d.x * 0.008;
            self.cam.pitch = (self.cam.pitch + d.y * 0.008).clamp(0.06, 1.53);
            busy = true;
        }
        if resp.drag_stopped() {
            self.grab = None;
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
            // Колесо с Alt — размер выбранного, с Shift — поворот (в правке); иначе — ближе и дальше.
            let notches = ui.input(|i| {
                i.events
                    .iter()
                    .map(|e| match e {
                        egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta, .. } => delta.y / 50.0,
                        egui::Event::MouseWheel { delta, .. } => delta.y,
                        _ => 0.0,
                    })
                    .sum::<f32>()
            });
            match self.picked.filter(|_| self.edit && (alt || shift)) {
                Some(pk) => {
                    if notches != 0.0 {
                        if self.wheel != Some((pk.storey, pk.index)) {
                            self.edits.push(Edit3::Begin(pk.storey));
                            self.wheel = Some((pk.storey, pk.index));
                        }
                        self.edits.push(if alt { Edit3::Scale(pk.storey, pk.index, 1.1f32.powf(notches)) } else { Edit3::Turn(pk.storey, pk.index, notches * TURN_STEP.0) });
                    }
                }
                None => {
                    let scroll = ui.input(|i| i.smooth_scroll_delta.y);
                    if scroll != 0.0 {
                        self.cam.dist = (self.cam.dist * (-scroll * 0.0018).exp()).clamp(1.2, 600.0);
                        busy = true;
                    }
                }
            }
        }
        let at = resp.interact_pointer_pos();
        let size = self.size;
        let inside = move |p: &[f32; 2]| p[0] >= 0.0 && p[1] >= 0.0 && p[0] <= size[0] && p[1] <= size[1];
        if self.edit {
            // Щелчок: предмет — выбрать; мимо — поставить с полки или снять выбор.
            if resp.clicked()
                && let Some(p) = at
            {
                match self.pick(rect, p) {
                    Some(j) => self.edits.push(Edit3::Select(Some((self.props[j].storey, self.props[j].index)))),
                    None => match self.ground_hit(rect, p, self.top).filter(|w| inside(w) && self.ghost.is_some()) {
                        Some(w) => self.edits.push(Edit3::Place(self.top, w, alt)),
                        None => self.edits.push(Edit3::Select(None)),
                    },
                }
            }
        } else if resp.double_clicked()
            && let Some(p) = at.and_then(|p| self.floor_hit(rect, p, self.top)).filter(inside)
        {
            self.place_token(p, self.top);
        } else if resp.clicked()
            && let Some(p) = at.and_then(|p| self.floor_hit(rect, p, self.top))
            && let Some(i) = self.door_near(self.top, p, CLICK_REACH)
        {
            self.toggle(self.top, i);
        }
        if resp.has_focus() {
            let (fwd, right) = self.cam.ground();
            let (mut mx, mut my) = (0.0f32, 0.0f32);
            let (up, down, left, rt, e, q, del, pgup, pgdn) = ui.input(|i| {
                (
                    i.key_down(Key::W) || i.key_down(Key::ArrowUp),
                    i.key_down(Key::S) || i.key_down(Key::ArrowDown),
                    i.key_down(Key::A) || i.key_down(Key::ArrowLeft),
                    i.key_down(Key::D) || i.key_down(Key::ArrowRight),
                    i.key_pressed(Key::E),
                    i.key_pressed(Key::Q),
                    i.key_pressed(Key::Delete) || i.key_pressed(Key::Backspace),
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
                let speed = SPEED * if shift { 2.0 } else { 1.0 } * dt / l;
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
            match self.picked.filter(|_| self.edit) {
                // Правка: Q и E вертят выбранный, Delete убирает.
                Some(pk) => {
                    let step = if shift { TURN_STEP.1 } else { TURN_STEP.0 };
                    for (on, deg) in [(q, -step), (e, step)] {
                        if on {
                            self.edits.push(Edit3::Begin(pk.storey));
                            self.edits.push(Edit3::Turn(pk.storey, pk.index, deg));
                        }
                    }
                    if del {
                        self.edits.push(Edit3::Begin(pk.storey));
                        self.edits.push(Edit3::Delete(pk.storey, pk.index));
                        self.grab = None;
                    }
                }
                None if !self.edit => {
                    if e && let Some(t) = self.token && let Some(i) = self.door_near(t.storey, t.at, REACH) {
                        self.toggle(t.storey, i);
                    }
                }
                None => {}
            }
            if pgup {
                self.change_storey(1);
            }
            if pgdn {
                self.change_storey(-1);
            }
        }
        if self.follow && let Some(t) = self.token {
            let goal = [t.at[0], Self::floor_y(t.storey) + t.feet + 0.6, t.at[1]];
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
        // Клавиши и щелчки сами будят кадр; дальше — пока что-то движется или ждёт правка.
        if busy || !self.edits.is_empty() {
            ui.ctx().request_repaint();
        }
    }

    fn set_edit(&mut self, on: bool) {
        self.edit = on;
        self.grab = None;
        self.hover = None;
    }

    /// Точка с привязкой к сетке, как у предметов на карте: `alt` меняет настройку на время.
    fn snapped(&self, p: [f32; 2], alt: bool) -> [f32; 2] {
        if self.snap != alt { [snap_half(p[0]), snap_half(p[1])] } else { p }
    }

    /// Луч из точки окна на землю этажа `k`: на пол, а где помост или склон — на их верх.
    fn ground_hit(&self, rect: Rect, pos: Pos2, k: usize) -> Option<[f32; 2]> {
        let f = self.floors.get(k)?;
        let mut w = self.floor_hit(rect, pos, k)?;
        for _ in 0..2 {
            w = self.plane_hit(rect, pos, Self::floor_y(k) + f.ground.at(w))?;
        }
        Some(w)
    }

    /// Поверх кадра: рамка выбранного (синяя, как на карте), того, что под курсором, и того,
    /// что поставит щелчок.
    fn overlay(&self, painter: &egui::Painter, rect: Rect, hover: Option<Pos2>) {
        let Some(vp) = self.vp else { return };
        let line = |a: [f32; 3], b: [f32; 3], stroke: Stroke| {
            if let (Some(p), Some(q)) = (render3d::project(&vp, rect, a), render3d::project(&vp, rect, b)) {
                painter.line_segment([p, q], stroke);
            }
        };
        let ring = |k: usize, corners: [[f32; 2]; 4], stroke: Stroke| {
            let ground = self.floors.get(k).map(|f| &f.ground);
            let lift = |c: [f32; 2]| [c[0], Self::floor_y(k) + ground.map_or(0.0, |g| g.at(c)) + 0.02, c[1]];
            for i in 0..4 {
                line(lift(corners[i]), lift(corners[(i + 1) % 4]), stroke);
            }
        };
        let blue = Stroke::new(2.0, Color32::from_rgb(120, 180, 255));
        if let Some(pk) = self.picked.filter(|pk| pk.storey <= self.top) {
            // Стоит в 3D — рамкой своей модели; в картинке пола — рамкой на полу.
            match self.prop_of(pk.storey, pk.index).filter(|p| self.shown(p)) {
                Some(p) => p.edges().into_iter().for_each(|(a, b)| line(a, b, blue)),
                None => ring(pk.storey, pk.corners, blue),
            }
        }
        if !self.edit {
            return;
        }
        let soft = Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 150));
        if let Some(p) = self.hover.and_then(|j| self.props.get(j)).filter(|p| self.picked.is_none_or(|pk| (pk.storey, pk.index) != (p.storey, p.index))) {
            p.edges().into_iter().for_each(|(a, b)| line(a, b, soft));
        }
        // Что поставит щелчок: рамка картинки с полки на земле под курсором.
        if let (Some(g), Some(h), None, None) = (self.ghost, hover, self.hover, self.grab)
            && let Some(w) = self.ground_hit(rect, h, self.top)
        {
            let alt = painter.ctx().input(|i| i.modifiers.alt);
            let c = self.snapped(w, alt);
            let (hw, hh) = (g.size[0] / 2.0, g.size[1] / 2.0);
            let (s, co) = g.rot.to_radians().sin_cos();
            let corners = [[-hw, -hh], [hw, -hh], [hw, hh], [-hw, hh]].map(|[x, y]| [c[0] + x * co - y * s, c[1] + x * s + y * co]);
            ring(self.top, corners, Stroke::new(1.5, ACCENT));
        }
    }
}

enum Action {
    None,
    Recapture,
}

/// Что уходит на видеокарту: узлы, их сетки, материалы, картинки — к ним добавляются образцы
/// предметов. `base` — сколько узлов у сцены уже есть: у добавки к залитой сцене номера её
/// узлов — после них, а номера материалов и картинок — свои, с нуля ([`render3d::Addition`]).
struct Parts {
    base: usize,
    nodes: Vec<NodeInfo>,
    meshes: Vec<Vec<(usize, crate::scene3d::Mesh)>>,
    materials: Vec<crate::scene3d::Material>,
    images: Vec<image::RgbaImage>,
}

impl Parts {
    /// Добавка к сцене из `base` узлов.
    fn after(base: usize) -> Parts {
        Parts { base, nodes: Vec::new(), meshes: Vec::new(), materials: Vec::new(), images: Vec::new() }
    }

    fn template(&mut self, prims: Vec<(usize, crate::scene3d::Mesh)>) -> usize {
        self.nodes.push(NodeInfo { part: Part::Floor, storey: 0, at: [0.0; 3], yaw: 0.0, door: None, template: true, roof: Vec::new() });
        self.meshes.push(prims);
        self.base + self.nodes.len() - 1
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

/// Предмет стоит в окне 3D сам (моделью или картинкой), а не в картинке пола — и правится в
/// 3D без снимка. Лестницы — нет: из них марши, это земля этажа.
fn live_prop(p: &Prop, map: &Map) -> bool {
    props3d::lift(p, map).is_some() && !crate::ground3d::flight_by_name(&p.asset)
}

/// Этаж без живых предметов: по нему видно, что поменялось то, что видно только после снимка.
fn still(map: &Map) -> Map {
    let mut m = map.clone();
    m.props.retain(|p| !live_prop(p, map));
    m
}

/// Предмет на этаже: этаж, номер на карте, сам предмет и как выходит в 3D.
type Plan = (usize, usize, Prop, props3d::Lift);

impl App {
    /// Этаж `k` — живой: у текущего это `map`.
    fn level_ref(&self, k: usize) -> &Map {
        if k == self.level { &self.map } else { &self.levels[k] }
    }

    /// Что стоит в 3D само, по этажам снизу вверх, на этаже — по слоям: без лестниц, ставших
    /// маршами (`flights` — их картинки и места при снимке), и без той же лестницы этажом выше
    /// (в дыре виден сам марш).
    fn plans_3d(&self, flights: &[Vec<(String, [f32; 2])>]) -> Vec<Plan> {
        let mut plans = Vec::new();
        for k in 0..self.levels.len() {
            let m = self.level_ref(k);
            let gone = |p: &Prop| flights.get(k).is_some_and(|f| f.iter().any(|(a, at)| *a == p.asset && *at == p.pos));
            let below = (k > 0).then(|| self.level_ref(k - 1));
            let over = |p: &Prop| below.is_some_and(|b| crate::ground3d::over_flight(p, b, &super::exporting::has_model));
            let mut here: Vec<Plan> = m.props.iter().enumerate().filter(|(_, p)| !gone(p) && !over(p)).filter_map(|(i, p)| props3d::lift(p, m).map(|l| (k, i, p.clone(), l))).collect();
            here.sort_by_key(|(_, _, p, _)| p.layer());
            plans.extend(here);
        }
        plans
    }

    /// Предметы в окно 3D: у кого есть модель — моделью (образец на модель), прочие — плоской
    /// картинкой (образец на картинку и подкраску), как их кладёт 3D Canvas. Образцы, которых ещё
    /// нет в `templates`, — в `parts`.
    fn props_3d(&mut self, templates: &mut Templates, plans: &[Plan], grounds: &[&scene3d::Ground], parts: &mut Parts) -> (Vec<PropAt>, usize) {
        let mut out = Vec::new();
        let mut modelled = 0;
        // Модели предметов — сначала: по ним видно, что на чём стоит.
        let mut found = Vec::with_capacity(plans.len());
        for (_, _, p, _) in plans {
            let model = match templates.models.get(&p.asset) {
                Some(m) => m.clone(),
                None => {
                    let m = self.prop_model(&p.asset).map(|m| (parts.add_model(&m), m));
                    templates.models.insert(p.asset.clone(), m.clone());
                    m
                }
            };
            found.push(model);
        }
        let mut heights = vec![0.0f32; plans.len()];
        for k in plans.iter().map(|(k, ..)| *k).collect::<std::collections::BTreeSet<_>>() {
            let here: Vec<usize> = (0..plans.len()).filter(|&i| plans[i].0 == k && plans[i].3 == props3d::Lift::Floor).collect();
            let items: Vec<props3d::Rest> = here
                .iter()
                .map(|&i| {
                    let p = &plans[i].2;
                    let size = [p.size[0] * p.scale, p.size[1] * p.scale];
                    let top = found[i].as_ref().and_then(|(_, m)| m.bounds().map(|[_, hi]| hi[1] * props3d::fit(m, size)[1]));
                    props3d::Rest { pos: p.pos, size, rot: p.rot, top }
                })
                .collect();
            for (i, h) in here.iter().zip(props3d::rests(&items)) {
                heights[*i] = h;
            }
        }
        for (i, (k, index, p, lift)) in plans.iter().enumerate() {
            // На помосте, ступени, склоне — на их высоте.
            let y0 = View3d::floor_y(*k) + grounds.get(*k).map_or(0.0, |g| g.at(p.pos)) + heights[i];
            let size = [p.size[0] * p.scale, p.size[1] * p.scale];
            let yaw = -p.rot.to_radians();
            let flip = if p.flip { -1.0 } else { 1.0 };
            if let Some((node, m)) = found[i].clone() {
                let s = props3d::fit(&m, size);
                let (at, scale) = ([p.pos[0], y0, p.pos[1]], [s[0] * flip, s[1], s[2]]);
                let bounds = m.bounds().unwrap_or([[-0.5, 0.0, -0.5], [0.5, 1.0, 0.5]]);
                out.push(PropAt { node, model: render3d::placed_scaled(at, yaw, scale), storey: *k, crown: false, bias: 0.0, index: *index, at, yaw, scale, bounds });
                modelled += 1;
                continue;
            }
            let key = (p.asset.clone(), p.tint);
            let node = match templates.cards.get(&key) {
                Some(n) => *n,
                None => {
                    let n = self.textures.rgba(&p.asset).ok().map(|img| parts.add_card(img, p.tint));
                    templates.cards.insert(key, n);
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
            let (at, scale) = ([p.pos[0], y0 + h, p.pos[1]], [size[0] * flip, 1.0, size[1]]);
            out.push(PropAt { node, model: render3d::placed_scaled(at, yaw, scale), storey: *k, crown: *lift == props3d::Lift::Crown, bias, index: *index, at, yaw, scale, bounds: [[-0.5, -0.02, -0.5], [0.5, 0.02, 0.5]] });
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
        // Где на экране середины предметов — их этаж и номер на карте.
        let spots = v.vp.map_or_else(Vec::new, |vp| {
            v.props
                .iter()
                .filter(|p| v.shown(p))
                .filter_map(|p| {
                    let [lo, hi] = p.bounds;
                    let mid = render3d::apply(&p.model, [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0, (lo[2] + hi[2]) / 2.0]);
                    render3d::project(&vp, v.rect, mid).map(|s| (p.storey, p.index, s))
                })
                .collect()
        });
        Some(Probe3d {
            rect: v.rect,
            yaw: v.cam.yaw,
            token: v.token.map(|t| t.at),
            open: v.doors.values().filter(|d| d.open).count(),
            painted: v.painted,
            error: v.renderer.error(),
            props: v.props.len(),
            modelled: v.modelled,
            storey: v.token.map_or(v.top, |t| t.storey),
            feet: v.token.map_or(0.0, |t| t.feet),
            roofs: v.nodes.iter().filter(|n| n.part == Part::Roof).count(),
            edit: v.edit,
            stale: v.stale,
            spots,
        })
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
        let mut covers = Vec::new();
        let mut pics: HashMap<String, image::RgbaImage> = HashMap::new();
        let map0 = self.export_level(e, 0);
        let size = [map0.width as f32, map0.height as f32];
        let first_time = time_of(map0);
        for (k, img) in e.done.iter().enumerate() {
            let map = self.export_level(e, k);
            layouts.push(export::layout(map, &map.room_edges()));
            let holes = e.whites.get(k).and_then(|w| w.as_ref()).and_then(|white| export::floor_from_pair(img, white));
            covers.push((k > 0).then(|| super::exporting::covers_of(holes.as_ref(), size)));
            let alpha = holes.is_some();
            let rgba = holes.unwrap_or_else(|| image::DynamicImage::ImageRgb8(img.clone()).to_rgba8());
            let key = format!("floor_{}", k + 1);
            floors.push(Some(scene3d::Picture { key: key.clone(), size: [rgba.width(), rgba.height()], alpha }));
            pics.insert(key, rgba);
        }
        let (mut storeys, flights) = self.storeys_3d(e, &layouts, floors, &covers, &mut pics);
        // Крыши — с картинками, как у тайлов крыш в выгрузке, на верху стен своего этажа.
        for (k, st) in storeys.iter_mut().enumerate() {
            let map = self.export_level(e, k);
            let Ok(mut roof_pics) = self.roof_pictures(map, e.px, None) else { continue };
            let rects = map.roofs.len();
            let relief = st.ground.relief.clone();
            let eave = |c: [f32; 2]| STOREY + relief.as_ref().map_or(0.0, |f| f.at(c));
            for id in 0..rects + map.outline_roofs.len() {
                let Some(pic) = roof_pics.remove(&id) else { continue };
                let Ok(img) = image::load_from_memory(&pic.bytes) else { continue };
                let key = format!("roof_{}_{}", k + 1, id + 1);
                let picture = scene3d::Picture { key: key.clone(), size: pic.size, alpha: true };
                let roof = match map.roofs.get(id) {
                    Some(r) => Some(crate::roof3d::rect(r, eave([(r.a[0] + r.b[0]) / 2.0, (r.a[1] + r.b[1]) / 2.0]), picture, pic.frame())),
                    None => {
                        let r = &map.outline_roofs[id - rects];
                        let (lo, hi) = r.bounds();
                        crate::roof3d::outline(r, eave([(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0]), picture, pic.frame(), ([0.0, 0.0], size))
                    }
                };
                if let Some(roof) = roof {
                    pics.insert(key, img.to_rgba8());
                    st.roofs.push(roof);
                }
            }
        }
        // Лестницы, ставшие маршами, — по картинке и месту: номера предметов сдвинутся с правкой.
        let flights: Vec<Vec<(String, [f32; 2])>> = flights.iter().enumerate().map(|(k, f)| f.iter().filter_map(|&i| self.export_level(e, k).props.get(i)).map(|p| (p.asset.clone(), p.pos)).collect()).collect();
        // Предметы, что не в картинке пола (они в выгрузке своими тайлами), — по этажам, снизу
        // вверх по слоям.
        let plans = self.plans_3d(&flights);
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
        // Узлы крыш идут в сцене по этажам и по порядку крыш — так же и их контуры.
        let mut outlines = storeys.iter().flat_map(|s| s.roofs.iter().map(|r| r.outline.clone()));
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
                roof: if n.part == Part::Roof { outlines.next().unwrap_or_default() } else { Vec::new() },
            })
            .collect();
        let meshes = scene.nodes.iter_mut().map(|n| std::mem::take(&mut n.prims)).collect();
        let mut parts = Parts { base: 0, nodes, meshes, materials: std::mem::take(&mut scene.materials), images };
        let mut templates = Templates::default();
        let (props, modelled) = {
            let grounds: Vec<&scene3d::Ground> = storeys.iter().map(|s| &s.ground).collect();
            self.props_3d(&mut templates, &plans, &grounds, &mut parts)
        };
        let Parts { nodes, meshes, materials, images, .. } = parts;
        let floors3: Vec<Floor3> = layouts
            .iter()
            .enumerate()
            .map(|(k, lay)| Floor3::new(k, lay, self.export_level(e, k).rooms.clone(), storeys[k].look.width.max(0.02) / 2.0, e.px as f32, storeys[k].ground.clone(), covers[k].clone()))
            .collect();
        let renderer = self.view3d.as_ref().map_or_else(Renderer::default, |v| v.renderer.clone());
        renderer.upload(Upload { nodes: meshes, materials, images, pawn: render3d::pawn() });
        let mut v = View3d::fresh(renderer, nodes, floors3, size, first_time);
        v.props = props;
        v.modelled = modelled;
        v.templates = templates;
        v.flights = flights;
        v.seen = (0..self.levels.len()).map(|k| self.level_ref(k).props.clone()).collect();
        v.base = (0..self.levels.len()).map(|k| still(self.level_ref(k))).collect();
        v.checked = self.rev;
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

    /// Окно 3D вслед за картой: живые предметы и их огни — заново, если карта их поменяла; что
    /// выбрано и что поставит щелчок; сверка, не пора ли снять заново (`released` — отпустили
    /// кнопку мыши: протяжка на холсте кончилась).
    fn view3d_follow(&mut self, v: &mut View3d, released: bool) {
        let n = v.floors.len();
        v.snap = self.settings.snap;
        v.picked = match self.sel {
            Some(Sel::Prop(i)) if i < self.map.props.len() && self.level < n => Some(Picked { storey: self.level, index: i, corners: self.map.props[i].corners() }),
            _ => None,
        };
        v.ghost = match (self.tool, self.object_asset.clone(), self.object_prefab) {
            (Tool::Object, _, Some(_)) => Some(Ghost { size: [1.0, 1.0], rot: self.object_rot }),
            (Tool::Object, Some(a), None) => {
                let s = self.asset_size(&a);
                Some(Ghost { size: [s[0] * self.object_scale, s[1] * self.object_scale], rot: self.object_rot })
            }
            _ => None,
        };
        // Другие этажи или размер — другая карта: только снимком.
        if self.levels.len() != n || [self.map.width as f32, self.map.height as f32] != v.size {
            v.stale = true;
            return;
        }
        let changed: Vec<usize> = (0..n).filter(|&k| v.seen.get(k).is_none_or(|s| *s != self.level_ref(k).props)).collect();
        if !changed.is_empty() {
            let plans = self.plans_3d(&v.flights);
            let mut parts = Parts::after(v.nodes.len());
            let (props, modelled) = {
                let grounds: Vec<&scene3d::Ground> = v.floors.iter().map(|f| &f.ground).collect();
                self.props_3d(&mut v.templates, &plans, &grounds, &mut parts)
            };
            v.props = props;
            v.modelled = modelled;
            v.hover = None;
            if !parts.nodes.is_empty() {
                let Parts { nodes, meshes, materials, images, .. } = parts;
                v.nodes.extend(nodes);
                v.renderer.add(render3d::Addition { nodes: meshes, materials, images });
            }
            v.seen = (0..n).map(|k| self.level_ref(k).props.clone()).collect();
            for k in changed {
                v.prop_lights(k, &export::prop_lights(self.level_ref(k)));
            }
        }
        if v.checked != self.rev || released {
            v.stale = (0..n).any(|k| v.base.get(k).is_none_or(|b| *b != still(self.level_ref(k))));
            v.checked = self.rev;
        }
    }

    /// Правки из окна 3D — карте, как правки мышью на холсте: с точкой отмены, на своём этаже.
    fn apply_3d(&mut self, edits: Vec<Edit3>) {
        for e in edits {
            match e {
                Edit3::Select(None) => {
                    self.sel = None;
                    self.picked.clear();
                }
                Edit3::Select(Some((k, i))) => {
                    self.switch_level(k);
                    if i < self.map.props.len() {
                        self.sel = Some(Sel::Prop(i));
                        self.picked.clear();
                        // Свойства выбранного — у «Выбора»; «Объект» продолжает ставить с полки.
                        if self.tool != Tool::Object && self.tool != Tool::Select {
                            self.set_tool(Tool::Select);
                        }
                    }
                }
                Edit3::Begin(k) => {
                    self.switch_level(k);
                    self.checkpoint();
                }
                Edit3::Move(k, i, pos) if k == self.level && i < self.map.props.len() => self.map.props[i].pos = pos,
                Edit3::Turn(k, i, deg) if k == self.level && i < self.map.props.len() => {
                    let r = &mut self.map.props[i].rot;
                    *r = ((*r + deg) % 360.0 + 360.0) % 360.0;
                }
                Edit3::Scale(k, i, f) if k == self.level && i < self.map.props.len() => {
                    let s = &mut self.map.props[i].scale;
                    *s = (*s * f).clamp(*SCALE_RANGE.start(), *SCALE_RANGE.end());
                }
                Edit3::Delete(k, i) if k == self.level && i < self.map.props.len() => {
                    self.map.props.remove(i);
                    self.sel = None;
                }
                Edit3::Place(k, p, alt) => {
                    self.switch_level(k);
                    match self.object_prefab {
                        Some(j) => {
                            self.place_prefab(j, p, alt);
                        }
                        None => {
                            self.place_prop(p, alt);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    /// Окно 3D, если открыто; закрытое отдаёт видеокарте память.
    pub(super) fn view3d_window(&mut self, ctx: &egui::Context) {
        let Some(mut v) = self.view3d.take() else { return };
        let released = ctx.input(|i| i.pointer.any_released());
        self.view3d_follow(&mut v, released);
        let mut open = v.open;
        let mut action = Action::None;
        egui::Window::new("Тестовая зона 3D").open(&mut open).pivot(egui::Align2::CENTER_CENTER).default_pos(ctx.content_rect().center()).default_size([960.0, 640.0]).min_size([420.0, 300.0]).resizable(true).collapsible(false).show(ctx, |ui| {
            action = v.ui(ui);
        });
        if !open {
            v.renderer.release(&ctx.layer_painter(egui::LayerId::background()));
            return;
        }
        let edits = std::mem::take(&mut v.edits);
        self.view3d = Some(v);
        self.apply_3d(edits);
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
        let floor = Floor3::new(0, &lay, Mask::for_cells(10, 10), 0.1, 100.0, scene3d::Ground::default(), None);
        let mut v = View3d::fresh(Renderer::default(), Vec::new(), vec![floor], [10.0, 10.0], Time::Night);
        v.relight_all();
        v
    }

    fn walk_to(v: &mut View3d, from: [f32; 2], to: [f32; 2]) -> [f32; 2] {
        v.token = Some(Token { at: from, storey: 0, feet: 0.0 });
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
        v.token = Some(Token { at: [5.0, 6.05], storey: 0, feet: 0.0 });
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
        v.token = Some(Token { at: [5.0, 8.5], storey: 0, feet: 0.0 });
        v.torch_step();
        let lit = light_at(&v, spot);
        assert!(lit > dark + 0.1, "факел светит: {dark} → {lit}");
        v.token = None;
        v.torch_step();
        assert!((light_at(&v, spot) - dark).abs() < 1e-4, "фишку убрали — темно");
        assert_eq!(v.floors[0].light.sources.len(), 1, "свой огонь фишки убран из света этажа");
    }

    fn rect(x0: f32, y0: f32, x1: f32, y1: f32, top: f32) -> scene3d::Block {
        scene3d::Block { poly: vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]], base: 0.0, top: scene3d::Top::Flat(top), side: [90, 90, 90], skin: None }
    }

    #[test]
    fn a_token_climbs_the_steps_onto_a_dais_but_not_its_wall() {
        let mut v = room();
        // Помост 0,6 у северной стены, три ступени к югу посередине.
        v.floors[0].ground.blocks = vec![rect(2.0, 2.0, 8.0, 3.5, 0.6), rect(4.5, 3.5, 5.5, 3.9, 0.45), rect(4.5, 3.9, 5.5, 4.3, 0.3), rect(4.5, 4.3, 5.5, 4.7, 0.15)];
        let at = walk_to(&mut v, [3.0, 5.0], [3.0, 2.5]);
        assert!(at[1] > 3.5 && v.token.unwrap().feet == 0.0, "в стенку помоста не вошла: {at:?}");
        let at = walk_to(&mut v, [5.0, 5.5], [5.0, 2.8]);
        assert!(at[1] < 3.5 && (v.token.unwrap().feet - 0.6).abs() < 1e-5, "по ступеням — на помост: {at:?} {}", v.token.unwrap().feet);
    }

    #[test]
    fn a_flight_takes_the_token_upstairs_and_back_down() {
        let lay = Layout { tents: Vec::new(), walls: Vec::new(), low: Vec::new(), cliffs: Vec::new(), portals: Vec::new(), lights: Vec::new(), zones: Vec::new(), passages: Vec::new() };
        // Марш с юга (y = 6) на север (y = 3) по x 4..6: семь ступеней до пола второго этажа.
        let steps: Vec<scene3d::Block> = (0..7).map(|i| rect(4.0, 6.0 - (i + 1) as f32 * 3.0 / 7.0, 6.0, 6.0 - i as f32 * 3.0 / 7.0, STOREY * (i + 1) as f32 / 7.0)).collect();
        let ground = scene3d::Ground { relief: None, blocks: steps };
        // Наверху над маршем — дыра.
        let cover = scene3d::Cover { w: 10, h: 10, per: 1.0, data: (0..100).map(|k| !((4..6).contains(&(k % 10)) && (3..6).contains(&(k / 10)))).collect() };
        let floors = vec![Floor3::new(0, &lay, Mask::for_cells(10, 10), 0.1, 100.0, ground, None), Floor3::new(1, &lay, Mask::for_cells(10, 10), 0.1, 100.0, scene3d::Ground::default(), Some(cover))];
        let mut v = View3d::fresh(Renderer::default(), Vec::new(), floors, [10.0, 10.0], Time::Day);
        v.token = Some(Token { at: [5.0, 7.0], storey: 0, feet: 0.0 });
        for _ in 0..100 {
            v.walk([0.0, -0.05]);
        }
        let t = v.token.unwrap();
        assert!(t.storey == 1 && t.at[1] < 3.0 && t.feet.abs() < 1e-5 && v.top == 1, "наверху: этаж {}, {:?}, {}", t.storey, t.at, t.feet);
        for _ in 0..100 {
            v.walk([0.0, 0.05]);
        }
        let t = v.token.unwrap();
        assert!(t.storey == 0 && t.at[1] > 6.0 && t.feet.abs() < 1e-5, "внизу: этаж {}, {:?}, {}", t.storey, t.at, t.feet);
    }

    /// Ящик 1 × 0,7 × 0,6 клетки моделью на полу в `p`, номер на карте `index`.
    fn crate_at(p: [f32; 2], index: usize) -> PropAt {
        let at = [p[0], 0.0, p[1]];
        PropAt { node: 0, model: render3d::placed(at, 0.0), storey: 0, crown: false, bias: 0.0, index, at, yaw: 0.0, scale: [1.0; 3], bounds: [[-0.5, 0.0, -0.3], [0.5, 0.7, 0.3]] }
    }

    #[test]
    fn a_ray_hits_the_turned_and_mirrored_box_of_a_prop() {
        // Узкая плита вдоль x, повёрнута на четверть: в мире — вдоль z.
        let mut p = crate_at([0.0, 0.0], 0);
        p.bounds = [[-1.0, 0.0, -0.1], [1.0, 0.2, 0.1]];
        p.yaw = std::f32::consts::FRAC_PI_2;
        p.model = render3d::placed(p.at, p.yaw);
        let down = [0.0, -1.0, 0.0];
        assert!(p.hit([0.0, 5.0, 0.8], down).is_some_and(|t| (t - 4.8).abs() < 1e-4), "вдоль z — попал в верх");
        assert!(p.hit([0.8, 5.0, 0.0], down).is_none(), "вдоль x — мимо");
        // Отражённый (растяжка меньше нуля) — та же рамка.
        p.scale = [-1.0, 1.0, 1.0];
        assert!(p.hit([0.0, 5.0, -0.8], down).is_some());
        // Рёбра рамки — по повороту: все концы у оси z.
        p.scale = [1.0; 3];
        assert!(p.edges().iter().all(|(a, b)| a[0].abs() < 0.11 && b[0].abs() < 0.11 && a[2].abs() <= 1.0 + 1e-5));
    }

    #[test]
    fn a_click_picks_the_prop_unless_a_wall_is_in_front_of_it() {
        let mut v = room();
        v.props = vec![crate_at([3.0, 4.0], 7)];
        let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 500.0));
        let aim = |v: &View3d| render3d::project(&v.frame(rect).vp.unwrap(), rect, [3.0, 0.35, 4.0]).unwrap();
        // Сверху, через стену комнаты, — ящик.
        v.cam = Orbit { target: [3.0, 0.35, 4.0], yaw: 0.0, pitch: 1.2, dist: 8.0 };
        assert_eq!(v.pick(rect, aim(&v)), Some(0));
        assert_eq!(v.prop_of(0, 7).map(|p| p.index), Some(7));
        // Снизу, с юга, луч идёт сквозь стену — не выбрать; стены по пояс — всё равно выше луча.
        v.cam = Orbit { target: [3.0, 0.35, 4.0], yaw: 0.0, pitch: 0.15, dist: 8.0 };
        assert_eq!(v.pick(rect, aim(&v)), None, "за стеной");
        // Мимо ящика — ничего.
        v.cam.pitch = 1.2;
        assert_eq!(v.pick(rect, aim(&v) + vec2(150.0, 0.0)), None);
    }

    #[test]
    fn the_light_of_a_moved_lamp_goes_with_it_and_the_torch_stays_last() {
        let mut v = room();
        let corner = [7.5, 5.5];
        let lit = light_at(&v, corner);
        v.token = Some(Token { at: [5.0, 3.0], storey: 0, feet: 0.0 });
        v.torch = false;
        let lamp = |at: [f32; 2]| export::Light { at, bright: 2.0, dim: 4.0, color: "#ffffff".into(), animation: String::new(), look: (0.5, 1, 0.0, 0.0), tempo: (5, 5) };
        v.prop_lights(0, &[lamp([2.5, 2.5])]);
        assert_eq!(v.floors[0].light.sources[0].at, [2.5, 2.5]);
        assert!(light_at(&v, corner) < lit - 0.1, "лампу унесли — в углу темнее: {lit} → {}", light_at(&v, corner));
        // С факелом фишки: огни предметов — заново, факел — последним.
        v.torch = true;
        v.torch_step();
        assert_eq!(v.floors[0].light.sources.len(), 2);
        v.prop_lights(0, &[lamp([5.0, 4.0])]);
        let s = &v.floors[0].light.sources;
        assert!(s.len() == 2 && s[0].at == [5.0, 4.0] && s[1].at == [5.0, 3.0], "{:?}", s.iter().map(|s| s.at).collect::<Vec<_>>());
        assert!((light_at(&v, corner) - lit).abs() < 0.05 || light_at(&v, corner) > lit, "лампа на месте — снова светло");
    }

    #[test]
    fn moving_furniture_needs_no_new_shot_but_moving_a_rug_does() {
        let mut map = Map::new(10, 10);
        let prop = |asset: &str| Prop { asset: asset.into(), pos: [3.0, 3.0], size: [1.0, 1.0], rot: 0.0, scale: 1.0, shadow: None, light: None, canopy: None, layer: None, tint: None, open: None, glow: None, sunk: None, flip: false };
        map.props.push(prop("pack:fa|textures/objects/Barrel_Wood_Dark_A_1x1.webp"));
        map.props.push(prop("pack:fa|textures/objects/Rug_Rectangle_C1_Colorable_3x2.webp"));
        let base = still(&map);
        map.props[0].pos = [6.0, 6.0];
        map.props[0].rot = 45.0;
        assert!(still(&map) == base, "бочка стоит в 3D сама — снимок не нужен");
        map.props[1].pos = [6.0, 6.0];
        assert!(still(&map) != base, "ковёр — в картинке пола");
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
