//! 3D-сцена из 2D-карты. Карта остаётся одна и рисуется сверху; здесь у её частей появляется
//! высота: пол этажа — плита с картинкой «пол без стен», стены — выдавленные ломаные толщиной
//! из вида стены, над дверью и окном — перемычка, под окном — подоконник со стеклом, створка
//! двери — отдельный узел на петле. Эту сцену пишет выгрузка в `.glb` для 3D Canvas (`glb`),
//! её же рисует тестовая зона — окно 3D в Картографе (`render3d`).
//!
//! Единицы — клетки: x — вправо, z — вниз по карте (на юг), y — вверх; клетка — 5 футов.
//! Так же лежит сцена 3D Canvas (x и z — точки холста / 1000), поэтому модель не зеркалится.

use crate::map::{PortalKind, Segment};

/// Клеток в футе: клетка — 5 футов.
pub const FOOT: f32 = 0.2;
/// Высота этажа: уровни Foundry в выгрузке идут через 10 футов.
pub const STOREY: f32 = 10.0 * FOOT;
/// Верх дверного и оконного проёма.
pub const DOOR: f32 = 7.0 * FOOT;
/// Подоконник.
pub const SILL: f32 = 3.0 * FOOT;
/// Низкая стена: парапет, зубцы.
pub const LOW: f32 = 3.0 * FOOT;
/// Толщина створки двери.
const LEAF: f32 = 0.08;
/// Толщина стекла.
const GLASS: f32 = 0.03;
/// Проём шире — не дверной проём под перемычкой, а разрыв стены (арка во двор, пролом).
const LINTEL_MAX: f32 = 2.5;
/// Боковая сторона стены: полоса-текстура повторяется по высоте не чаще, чем через столько
/// клеток (полоса высотой в толщину стены слишком мелкая для стороны высотой в этаж).
const SIDE_PERIOD: f32 = 0.5;
/// Створка открывается на четверть оборота.
const SWING: f32 = std::f32::consts::FRAC_PI_2;

/// Дерево створок, пока у дверей нет своих моделей.
const WOOD: [u8; 3] = [104, 74, 48];
/// Стекло окна.
const GLASS_TINT: [u8; 3] = [170, 200, 220];

/// Чем служит узел: по этому выгрузка ставит метки 3D Canvas.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Part {
    Floor,
    Wall,
    Glass,
    Door,
}

/// Картинка: ключ, по которому её найдёт выгрузка (ассет или пол этажа), размер и есть ли
/// прозрачность.
#[derive(Clone, Debug, PartialEq)]
pub struct Picture {
    pub key: String,
    pub size: [u32; 2],
    pub alpha: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Look {
    /// Номер в `Scene3d::pictures`.
    Picture(usize),
    Color([u8; 3]),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Alpha {
    Opaque,
    /// Вырез по альфе: дыры пола верхнего этажа.
    Mask,
    /// Полупрозрачное: стекло.
    Blend,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Material {
    pub look: Look,
    pub alpha: Alpha,
    pub two_sided: bool,
    /// Узор повторяется плиткой (стены); иначе картинка кладётся один раз (пол этажа).
    pub repeat: bool,
    pub opacity: f32,
}

/// Порядок полей — как в буфере вершин окна 3D (`render3d`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vertex {
    pub p: [f32; 3],
    pub n: [f32; 3],
    pub uv: [f32; 2],
}

#[derive(Clone, Debug, Default)]
pub struct Mesh {
    pub v: Vec<Vertex>,
    pub idx: Vec<u32>,
}

/// Створка двери: `id` — дверь Foundry, к которой она привязана; `swing` — на сколько
/// поворачивается, открываясь (радианы, знак — в какую сторону).
#[derive(Clone, Debug, PartialEq)]
pub struct Door {
    pub id: String,
    pub swing: f32,
}

pub struct Node {
    pub name: String,
    pub part: Part,
    pub storey: usize,
    /// Начало узла; у створки — петля.
    pub at: [f32; 3],
    /// Поворот вокруг вертикали, радианы, как в glTF: ось x узла уходит в (cos, 0, −sin).
    pub yaw: f32,
    /// Куски по материалам: номер в `Scene3d::materials` и сетка.
    pub prims: Vec<(usize, Mesh)>,
    pub door: Option<Door>,
}

#[derive(Default)]
pub struct Scene3d {
    pub nodes: Vec<Node>,
    pub materials: Vec<Material>,
    pub pictures: Vec<Picture>,
}

/// Вид стен этажа: толщина в клетках и текстура-полоса (как в 2D: лежит вдоль стены,
/// высотой в толщину) или цвет.
#[derive(Clone, Debug, PartialEq)]
pub struct WallLook {
    pub width: f32,
    pub strip: Option<Picture>,
    /// Бок стены: середина полосы без кромок. Сама полоса — вид сверху, с узором или
    /// бордюром по краям, и повторённая по высоте она ложилась бы на бок слоёным пирогом.
    pub side: Option<Picture>,
    pub color: [u8; 3],
}

/// Проём, который выгрузка отдаёт Foundry: дверь, двойная, окно или тайный ход. `id` — стена
/// Foundry с этой дверью.
#[derive(Clone, Debug, PartialEq)]
pub struct Opening {
    pub seg: Segment,
    pub kind: PortalKind,
    pub id: String,
}

/// Этаж для построения.
pub struct Storey {
    /// Картинка «пол без стен» во всю карту; на верхних этажах — с дырами.
    pub floor: Option<Picture>,
    /// Сплошные стены — ломаные в клетках, проёмы уже вырезаны (`export::Layout::walls`).
    pub walls: Vec<Vec<[f32; 2]>>,
    /// Низкие стены.
    pub low: Vec<Vec<[f32; 2]>>,
    /// Двери, окна, тайные ходы.
    pub openings: Vec<Opening>,
    /// Пустые проёмы (`PortalKind::Opening`): над узким — перемычка.
    pub gaps: Vec<Segment>,
    pub look: WallLook,
}

impl Scene3d {
    fn picture(&mut self, p: &Picture) -> usize {
        if let Some(i) = self.pictures.iter().position(|q| q.key == p.key) {
            return i;
        }
        self.pictures.push(p.clone());
        self.pictures.len() - 1
    }

    fn material(&mut self, m: Material) -> usize {
        if let Some(i) = self.materials.iter().position(|q| *q == m) {
            return i;
        }
        self.materials.push(m);
        self.materials.len() - 1
    }

    /// Рамка всей сцены в клетках: `[min, max]` по x, y, z (с поворотом и сдвигом узлов).
    pub fn bounds(&self) -> Option<[[f32; 3]; 2]> {
        let mut lo = [f32::MAX; 3];
        let mut hi = [f32::MIN; 3];
        for n in &self.nodes {
            let (s, c) = n.yaw.sin_cos();
            for (_, m) in &n.prims {
                for v in &m.v {
                    let p = [n.at[0] + c * v.p[0] + s * v.p[2], n.at[1] + v.p[1], n.at[2] - s * v.p[0] + c * v.p[2]];
                    for k in 0..3 {
                        lo[k] = lo[k].min(p[k]);
                        hi[k] = hi[k].max(p[k]);
                    }
                }
            }
        }
        (lo[0] <= hi[0]).then_some([lo, hi])
    }
}

/// Как ложится текстура на стену: длина повтора вдоль на верхе и на сторонах, по высоте — на
/// сторонах (в клетках).
#[derive(Clone, Copy)]
struct Tiling {
    top: f32,
    side_u: f32,
    side_v: f32,
}

impl Tiling {
    fn of(look: &WallLook) -> Self {
        let width = look.width.max(0.01);
        let aspect = |p: &Option<Picture>| p.as_ref().map(|p| p.size[0] as f32 / p.size[1].max(1) as f32);
        let top = aspect(&look.strip).unwrap_or(1.0);
        let side = aspect(&look.side).unwrap_or(top);
        let side_v = width.max(SIDE_PERIOD);
        Self { top: width * top, side_u: side_v * side, side_v }
    }
}

/// Стена кусками: верх (вид сверху, как на 2D-карте) и бока с торцами — у них разные картинки.
#[derive(Default)]
struct Shell {
    top: Mesh,
    side: Mesh,
}

impl Shell {
    /// Всё одной сеткой — для створки и стекла, у них один материал.
    fn into_one(mut self) -> Mesh {
        let base = self.top.v.len() as u32;
        self.top.v.extend(self.side.v);
        self.top.idx.extend(self.side.idx.into_iter().map(|i| i + base));
        self.top
    }
}

/// Построить 3D-сцену карты `size` (клетки) по этажам снизу вверх.
pub fn build(size: [f32; 2], storeys: &[Storey]) -> Scene3d {
    let mut s = Scene3d::default();
    for (k, st) in storeys.iter().enumerate() {
        let y0 = k as f32 * STOREY;
        let y1 = y0 + STOREY;

        // Пол — плита во всю карту с картинкой этажа (дыры верхнего этажа — вырезом по альфе).
        let floor_mat = match &st.floor {
            Some(p) => {
                let look = Look::Picture(s.picture(p));
                s.material(Material { look, alpha: if p.alpha { Alpha::Mask } else { Alpha::Opaque }, two_sided: true, repeat: false, opacity: 1.0 })
            }
            None => s.material(Material { look: Look::Color([96, 90, 84]), alpha: Alpha::Opaque, two_sided: true, repeat: false, opacity: 1.0 }),
        };
        let mut floor = Mesh::default();
        let (w, h) = (size[0], size[1]);
        quad(&mut floor, [[0.0, y0, 0.0], [w, y0, 0.0], [w, y0, h], [0.0, y0, h]], [0.0, 1.0, 0.0], [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]);
        s.nodes.push(Node { name: format!("floor_{}", k + 1), part: Part::Floor, storey: k, at: [0.0; 3], yaw: 0.0, prims: vec![(floor_mat, floor)], door: None });

        // Стены, перемычки и подоконники — одна сетка этажа.
        let look = &st.look;
        let tiling = Tiling::of(look);
        let half = look.width.max(0.02) / 2.0;
        let wall_mat = match &look.strip {
            Some(p) => {
                let pic = s.picture(p);
                s.material(Material { look: Look::Picture(pic), alpha: if p.alpha { Alpha::Mask } else { Alpha::Opaque }, two_sided: false, repeat: true, opacity: 1.0 })
            }
            None => s.material(Material { look: Look::Color(look.color), alpha: Alpha::Opaque, two_sided: false, repeat: true, opacity: 1.0 }),
        };
        let side_mat = match &look.side {
            Some(p) => {
                let pic = s.picture(p);
                s.material(Material { look: Look::Picture(pic), alpha: if p.alpha { Alpha::Mask } else { Alpha::Opaque }, two_sided: false, repeat: true, opacity: 1.0 })
            }
            None => wall_mat,
        };
        let mut walls = Shell::default();
        for line in &st.walls {
            extrude(&mut walls, line, half, y0, y1, tiling);
        }
        for line in &st.low {
            extrude(&mut walls, line, half, y0, y0 + LOW, tiling);
        }
        for g in &st.gaps {
            if dist(g.0, g.1) <= LINTEL_MAX {
                extrude(&mut walls, &[g.0, g.1], half, y0 + DOOR, y1, tiling);
            }
        }

        let wood = s.material(Material { look: Look::Color(WOOD), alpha: Alpha::Opaque, two_sided: false, repeat: true, opacity: 1.0 });
        let glass = s.material(Material { look: Look::Color(GLASS_TINT), alpha: Alpha::Blend, two_sided: true, repeat: true, opacity: 0.3 });
        for (i, o) in st.openings.iter().enumerate() {
            let (a, b) = o.seg;
            match o.kind {
                PortalKind::Door => {
                    extrude(&mut walls, &[a, b], half, y0 + DOOR, y1, tiling);
                    s.nodes.push(leaf(format!("door_{}_{}", k + 1, i + 1), k, a, b, y0, DOOR, wood, Door { id: o.id.clone(), swing: SWING }));
                }
                PortalKind::DoubleDoor => {
                    extrude(&mut walls, &[a, b], half, y0 + DOOR, y1, tiling);
                    let mid = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
                    s.nodes.push(leaf(format!("door_{}_{}a", k + 1, i + 1), k, a, mid, y0, DOOR, wood, Door { id: format!("{}_a", o.id), swing: SWING }));
                    s.nodes.push(leaf(format!("door_{}_{}b", k + 1, i + 1), k, b, mid, y0, DOOR, wood, Door { id: format!("{}_b", o.id), swing: -SWING }));
                }
                // Тайный ход снаружи неотличим от стены: створка во всю высоту, из стены.
                PortalKind::Secret => {
                    s.nodes.push(leaf(format!("secret_{}_{}", k + 1, i + 1), k, a, b, y0, STOREY, side_mat, Door { id: o.id.clone(), swing: SWING }));
                }
                PortalKind::Window => {
                    extrude(&mut walls, &[a, b], half, y0, y0 + SILL, tiling);
                    extrude(&mut walls, &[a, b], half, y0 + DOOR, y1, tiling);
                    let mut pane = Shell::default();
                    extrude(&mut pane, &[a, b], GLASS / 2.0, y0 + SILL, y0 + DOOR, tiling);
                    s.nodes.push(Node { name: format!("window_{}_{}", k + 1, i + 1), part: Part::Glass, storey: k, at: [0.0; 3], yaw: 0.0, prims: vec![(glass, pane.into_one())], door: None });
                }
                PortalKind::Opening => {
                    if dist(a, b) <= LINTEL_MAX {
                        extrude(&mut walls, &[a, b], half, y0 + DOOR, y1, tiling);
                    }
                }
            }
        }
        let prims = if side_mat == wall_mat { vec![(wall_mat, walls.into_one())] } else { vec![(wall_mat, walls.top), (side_mat, walls.side)] };
        let prims: Vec<(usize, Mesh)> = prims.into_iter().filter(|(_, m)| !m.idx.is_empty()).collect();
        if !prims.is_empty() {
            s.nodes.push(Node { name: format!("walls_{}", k + 1), part: Part::Wall, storey: k, at: [0.0; 3], yaw: 0.0, prims, door: None });
        }
    }
    s
}

/// Створка от петли `hinge` к `to` на высоте `y0`, высотой `tall`: узел с началом на петле,
/// сетка вдоль его оси x — так 3D Canvas поворачивает её вокруг петли.
#[allow(clippy::too_many_arguments)]
fn leaf(name: String, storey: usize, hinge: [f32; 2], to: [f32; 2], y0: f32, tall: f32, mat: usize, door: Door) -> Node {
    let (dx, dz) = (to[0] - hinge[0], to[1] - hinge[1]);
    let len = dx.hypot(dz).max(0.01);
    let mut m = Shell::default();
    let tiling = Tiling { top: 1.0, side_u: 1.0, side_v: 1.0 };
    // Чуть короче проёма и ниже притолоки: не трётся о косяк, когда поворачивается.
    extrude(&mut m, &[[0.02, 0.0], [len - 0.02, 0.0]], LEAF / 2.0, 0.0, tall - 0.02, tiling);
    Node { name, part: Part::Door, storey, at: [hinge[0], y0, hinge[1]], yaw: (-dz).atan2(dx), prims: vec![(mat, m.into_one())], door: Some(door) }
}

fn dist(a: [f32; 2], b: [f32; 2]) -> f32 {
    (b[0] - a[0]).hypot(b[1] - a[1])
}

fn unit(v: [f32; 2]) -> [f32; 2] {
    let l = v[0].hypot(v[1]);
    if l < 1e-9 { [0.0, 0.0] } else { [v[0] / l, v[1] / l] }
}

/// Нормаль влево по ходу (в плоскости x–z).
fn left(d: [f32; 2]) -> [f32; 2] {
    [d[1], -d[0]]
}

/// Выдавить ломаную в стену толщиной `2·half` от `y0` до `y1`: стороны, верх и торцы у
/// свободных концов; на изломах — скос (углы без щелей и нахлёстов). Замкнутая — если первая
/// точка совпадает с последней.
fn extrude(m: &mut Shell, line: &[[f32; 2]], half: f32, y0: f32, y1: f32, t: Tiling) {
    let mut pts: Vec<[f32; 2]> = Vec::with_capacity(line.len());
    for &p in line {
        if pts.last().is_none_or(|&q| dist(q, p) > 1e-5) {
            pts.push(p);
        }
    }
    let closed = pts.len() > 2 && dist(pts[0], pts[pts.len() - 1]) < 1e-4;
    if closed {
        pts.pop();
    }
    let n = pts.len();
    if n < 2 || y1 <= y0 {
        return;
    }
    let dir = |i: usize, j: usize| unit([pts[j][0] - pts[i][0], pts[j][1] - pts[i][1]]);
    // Сдвиг влево у каждой вершины: на концах — по нормали отрезка, на изломе — по биссектрисе,
    // длиной, чтобы стороны остались на `half` от оси (не длиннее трёх толщин у острых углов).
    let offs: Vec<[f32; 2]> = (0..n)
        .map(|i| {
            let prev = if i > 0 { Some(dir(i - 1, i)) } else if closed { Some(dir(n - 1, 0)) } else { None };
            let next = if i + 1 < n { Some(dir(i, i + 1)) } else if closed { Some(dir(n - 1, 0)) } else { None };
            match (prev, next) {
                (Some(a), Some(b)) => {
                    let (na, nb) = (left(a), left(b));
                    let mut mid = unit([na[0] + nb[0], na[1] + nb[1]]);
                    if mid == [0.0, 0.0] {
                        mid = na;
                    }
                    let k = half / (mid[0] * na[0] + mid[1] * na[1]).max(1.0 / 3.0);
                    [mid[0] * k, mid[1] * k]
                }
                (Some(a), None) | (None, Some(a)) => {
                    let l = left(a);
                    [l[0] * half, l[1] * half]
                }
                (None, None) => [0.0, 0.0],
            }
        })
        .collect();
    let l = |i: usize| [pts[i][0] + offs[i][0], pts[i][1] + offs[i][1]];
    let r = |i: usize| [pts[i][0] - offs[i][0], pts[i][1] - offs[i][1]];
    let at = |p: [f32; 2], y: f32| [p[0], y, p[1]];
    let segs = if closed { n } else { n - 1 };
    let mut run = 0.0;
    for s in 0..segs {
        let (i, j) = (s, (s + 1) % n);
        let d = dir(i, j);
        let nl = left(d);
        let len = dist(pts[i], pts[j]);
        let (u0, u1) = (run, run + len);
        run = u1;
        let (su0, su1) = (u0 / t.side_u, u1 / t.side_u);
        let (sv0, sv1) = ((y1 - y0) / t.side_v, 0.0);
        quad(&mut m.side, [at(l(i), y0), at(l(j), y0), at(l(j), y1), at(l(i), y1)], [nl[0], 0.0, nl[1]], [[su0, sv0], [su1, sv0], [su1, sv1], [su0, sv1]]);
        quad(&mut m.side, [at(r(j), y0), at(r(i), y0), at(r(i), y1), at(r(j), y1)], [-nl[0], 0.0, -nl[1]], [[su1, sv0], [su0, sv0], [su0, sv1], [su1, sv1]]);
        let (tu0, tu1) = (u0 / t.top, u1 / t.top);
        quad(&mut m.top, [at(l(i), y1), at(l(j), y1), at(r(j), y1), at(r(i), y1)], [0.0, 1.0, 0.0], [[tu0, 0.0], [tu1, 0.0], [tu1, 1.0], [tu0, 1.0]]);
    }
    if !closed {
        let h = (y1 - y0) / t.side_v;
        let w = 2.0 * half / t.side_u;
        let d0 = dir(0, 1);
        quad(&mut m.side, [at(r(0), y0), at(l(0), y0), at(l(0), y1), at(r(0), y1)], [-d0[0], 0.0, -d0[1]], [[0.0, h], [w, h], [w, 0.0], [0.0, 0.0]]);
        let d1 = dir(n - 2, n - 1);
        quad(&mut m.side, [at(l(n - 1), y0), at(r(n - 1), y0), at(r(n - 1), y1), at(l(n - 1), y1)], [d1[0], 0.0, d1[1]], [[0.0, h], [w, h], [w, 0.0], [0.0, 0.0]]);
    }
}

/// Четырёхугольник с нормалью `n`: обход вершин подбирается так, чтобы лицевая сторона
/// смотрела по `n` (glTF: лицо — обход против часовой, если смотреть на него).
fn quad(m: &mut Mesh, p: [[f32; 3]; 4], n: [f32; 3], uv: [[f32; 2]; 4]) {
    let base = m.v.len() as u32;
    for k in 0..4 {
        m.v.push(Vertex { p: p[k], n, uv: uv[k] });
    }
    let e1 = [p[1][0] - p[0][0], p[1][1] - p[0][1], p[1][2] - p[0][2]];
    let e2 = [p[2][0] - p[0][0], p[2][1] - p[0][1], p[2][2] - p[0][2]];
    let c = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
    if c[0] * n[0] + c[1] * n[1] + c[2] * n[2] >= 0.0 {
        m.idx.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    } else {
        m.idx.extend([base, base + 2, base + 1, base, base + 3, base + 2]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(width: f32) -> WallLook {
        WallLook { width, strip: None, side: None, color: [70, 60, 50] }
    }

    fn storey(walls: Vec<Vec<[f32; 2]>>, openings: Vec<Opening>) -> Storey {
        Storey { floor: None, walls, low: Vec::new(), openings, gaps: Vec::new(), look: plain(0.25) }
    }

    /// Все треугольники сетки смотрят лицом по нормалям своих вершин.
    fn faces_follow_normals(m: &Mesh) -> bool {
        m.idx.chunks(3).all(|t| {
            let p = |k: usize| m.v[t[k] as usize].p;
            let (a, b, c) = (p(0), p(1), p(2));
            let e1 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let e2 = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
            let x = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
            let n = m.v[t[0] as usize].n;
            x[0] * n[0] + x[1] * n[1] + x[2] * n[2] > 0.0
        })
    }

    #[test]
    fn a_wall_is_a_box_of_its_thickness_and_a_storey_high() {
        let s = build([4.0, 4.0], &[storey(vec![vec![[1.0, 2.0], [3.0, 2.0]]], Vec::new())]);
        let walls = s.nodes.iter().find(|n| n.part == Part::Wall).unwrap();
        let m = &walls.prims[0].1;
        let (lo, hi) = m.v.iter().fold(([f32::MAX; 3], [f32::MIN; 3]), |(lo, hi), v| ([0, 1, 2].map(|k| lo[k].min(v.p[k])), [0, 1, 2].map(|k| hi[k].max(v.p[k]))));
        assert_eq!(lo, [1.0, 0.0, 1.875]);
        assert_eq!(hi, [3.0, STOREY, 2.125]);
        assert!(faces_follow_normals(m));
        // Две стороны, верх и два торца.
        assert_eq!(m.idx.len(), 5 * 6);
    }

    #[test]
    fn a_corner_is_mitred_without_a_gap() {
        let s = build([4.0, 4.0], &[storey(vec![vec![[1.0, 1.0], [3.0, 1.0], [3.0, 3.0]]], Vec::new())]);
        let m = &s.nodes.iter().find(|n| n.part == Part::Wall).unwrap().prims[0].1;
        // Внешний угол — на четверть толщины снаружи по обеим осям, внутренний — внутри.
        let has = |x: f32, z: f32| m.v.iter().any(|v| (v.p[0] - x).abs() < 1e-5 && (v.p[2] - z).abs() < 1e-5);
        assert!(has(3.125, 0.875), "внешний угол");
        assert!(has(2.875, 1.125), "внутренний угол");
        assert!(faces_follow_normals(m));
    }

    #[test]
    fn a_closed_room_has_no_end_caps() {
        let ring = vec![[1.0, 1.0], [3.0, 1.0], [3.0, 3.0], [1.0, 3.0], [1.0, 1.0]];
        let s = build([4.0, 4.0], &[storey(vec![ring], Vec::new())]);
        let m = &s.nodes.iter().find(|n| n.part == Part::Wall).unwrap().prims[0].1;
        // Четыре отрезка по три грани, без торцов.
        assert_eq!(m.idx.len(), 4 * 3 * 6);
        assert!(faces_follow_normals(m));
    }

    #[test]
    fn a_door_gets_a_lintel_and_a_leaf_on_its_hinge() {
        let door = Opening { seg: ([2.0, 1.0], [3.0, 1.0]), kind: PortalKind::Door, id: "w7".into() };
        let s = build([4.0, 4.0], &[storey(vec![vec![[0.0, 1.0], [2.0, 1.0]], vec![[3.0, 1.0], [4.0, 1.0]]], vec![door])]);
        let leaf = s.nodes.iter().find(|n| n.part == Part::Door).unwrap();
        assert_eq!(leaf.at, [2.0, 0.0, 1.0]);
        assert_eq!(leaf.door.as_ref().unwrap().id, "w7");
        assert!(leaf.yaw.abs() < 1e-6, "створка вдоль оси x");
        let [lo, hi] = s.bounds().unwrap();
        assert_eq!(hi[1], STOREY);
        assert!(lo[0] <= 0.0 && hi[0] >= 4.0);
        // Перемычка над проёмом: в сетке стен есть верх двери.
        let walls = &s.nodes.iter().find(|n| n.part == Part::Wall).unwrap().prims[0].1;
        assert!(walls.v.iter().any(|v| (v.p[1] - DOOR).abs() < 1e-6 && (2.0..=3.0).contains(&v.p[0])));
    }

    /// Вершины узла на месте в сцене (поворот вокруг вертикали и сдвиг).
    fn placed(n: &Node) -> Vec<[f32; 3]> {
        let (s, c) = n.yaw.sin_cos();
        n.prims.iter().flat_map(|(_, m)| m.v.iter()).map(|v| [n.at[0] + c * v.p[0] + s * v.p[2], n.at[1] + v.p[1], n.at[2] - s * v.p[0] + c * v.p[2]]).collect()
    }

    #[test]
    fn a_leaf_along_the_map_y_axis_turns_to_face_it() {
        let door = Opening { seg: ([1.0, 1.0], [1.0, 2.0]), kind: PortalKind::Door, id: "w1".into() };
        let s = build([4.0, 4.0], &[storey(Vec::new(), vec![door])]);
        let leaf = s.nodes.iter().find(|n| n.part == Part::Door).unwrap();
        // Створка от петли (1, 1) вниз по карте до (1, 2): после поворота узла она там и лежит.
        assert!((leaf.yaw + std::f32::consts::FRAC_PI_2).abs() < 1e-6);
        let pts = placed(leaf);
        assert!(pts.iter().all(|p| p[2] > 1.0 && p[2] < 2.0 && (p[0] - 1.0).abs() <= LEAF), "{pts:?}");
    }

    #[test]
    fn a_double_door_swings_both_leaves_the_same_way() {
        let door = Opening { seg: ([0.0, 1.0], [2.0, 1.0]), kind: PortalKind::DoubleDoor, id: "w3".into() };
        let s = build([4.0, 4.0], &[storey(Vec::new(), vec![door])]);
        let leaves: Vec<&Node> = s.nodes.iter().filter(|n| n.part == Part::Door).collect();
        assert_eq!(leaves.len(), 2);
        assert_eq!(leaves[0].door.as_ref().unwrap().id, "w3_a");
        assert_eq!(leaves[1].at, [2.0, 0.0, 1.0]);
        // Вторая смотрит навстречу первой и поворачивается в обратную сторону: обе — в одну сторону стены.
        assert!((leaves[1].yaw.abs() - std::f32::consts::PI).abs() < 1e-5);
        assert_eq!(leaves[0].door.as_ref().unwrap().swing, -leaves[1].door.as_ref().unwrap().swing);
    }

    #[test]
    fn a_window_has_a_sill_a_lintel_and_glass_between() {
        let win = Opening { seg: ([1.0, 1.0], [2.0, 1.0]), kind: PortalKind::Window, id: "w2".into() };
        let s = build([4.0, 4.0], &[storey(Vec::new(), vec![win])]);
        let glass = s.nodes.iter().find(|n| n.part == Part::Glass).unwrap();
        let ys: Vec<f32> = glass.prims[0].1.v.iter().map(|v| v.p[1]).collect();
        assert!(ys.iter().all(|&y| (SILL - 1e-5..=DOOR + 1e-5).contains(&y)));
        assert_eq!(s.materials[glass.prims[0].0].alpha, Alpha::Blend);
        let walls = &s.nodes.iter().find(|n| n.part == Part::Wall).unwrap().prims[0].1;
        assert!(walls.v.iter().any(|v| (v.p[1] - SILL).abs() < 1e-6));
        assert!(walls.v.iter().any(|v| (v.p[1] - DOOR).abs() < 1e-6));
    }

    #[test]
    fn upper_storeys_stand_a_storey_higher_and_their_floor_keeps_its_holes() {
        let mut up = storey(vec![vec![[1.0, 1.0], [2.0, 1.0]]], Vec::new());
        up.floor = Some(Picture { key: "floor_2".into(), size: [400, 400], alpha: true });
        let s = build([4.0, 4.0], &[storey(Vec::new(), Vec::new()), up]);
        let floor2 = s.nodes.iter().find(|n| n.name == "floor_2").unwrap();
        assert!(floor2.prims[0].1.v.iter().all(|v| v.p[1] == STOREY));
        assert_eq!(s.materials[floor2.prims[0].0].alpha, Alpha::Mask);
        let [_, hi] = s.bounds().unwrap();
        assert_eq!(hi[1], 2.0 * STOREY);
    }

    #[test]
    fn the_sides_get_their_own_picture_when_there_is_one() {
        let strip = Picture { key: "stone".into(), size: [1024, 256], alpha: false };
        let side = Picture { key: "stone#side".into(), size: [1024, 128], alpha: false };
        let look = WallLook { width: 0.25, strip: Some(strip), side: Some(side), color: [0; 3] };
        let st = Storey { floor: None, walls: vec![vec![[0.0, 1.0], [2.0, 1.0]]], low: Vec::new(), openings: Vec::new(), gaps: Vec::new(), look };
        let s = build([4.0, 4.0], &[st]);
        let walls = s.nodes.iter().find(|n| n.part == Part::Wall).unwrap();
        assert_eq!(walls.prims.len(), 2);
        let (top, sides) = (&walls.prims[0], &walls.prims[1]);
        assert!(top.1.v.iter().all(|v| v.n == [0.0, 1.0, 0.0]), "верх — только верх");
        assert!(sides.1.v.iter().all(|v| v.n[1] == 0.0), "бока и торцы");
        assert_eq!(s.materials[sides.0].look, Look::Picture(1));
        // Бок: по высоте повтор через полклетки, по длине — в пропорции своей картинки (8 : 1).
        let umax = sides.1.v.iter().map(|v| v.uv[0]).fold(f32::MIN, f32::max);
        assert!((umax - 2.0 / (0.5 * 8.0)).abs() < 1e-5, "{umax}");
    }

    #[test]
    fn the_wall_strip_lies_along_the_top_like_on_the_2d_map() {
        let look = WallLook { width: 0.25, strip: Some(Picture { key: "stone".into(), size: [1024, 256], alpha: false }), side: None, color: [0; 3] };
        let st = Storey { floor: None, walls: vec![vec![[0.0, 1.0], [2.0, 1.0]]], low: Vec::new(), openings: Vec::new(), gaps: Vec::new(), look };
        let s = build([4.0, 4.0], &[st]);
        let walls = s.nodes.iter().find(|n| n.part == Part::Wall).unwrap();
        let m = &walls.prims[0].1;
        // Верх: полоса высотой в толщину (0,25) и длиной в четыре толщины — на 2 клетки два повтора.
        let top: Vec<&Vertex> = m.v.iter().filter(|v| v.n == [0.0, 1.0, 0.0]).collect();
        let umax = top.iter().map(|v| v.uv[0]).fold(f32::MIN, f32::max);
        assert!((umax - 2.0).abs() < 1e-5, "{umax}");
        assert!(top.iter().all(|v| v.uv[1] == 0.0 || v.uv[1] == 1.0));
        assert!(s.materials[walls.prims[0].0].repeat);
    }
}
