//! 3D-сцена из 2D-карты. Карта остаётся одна и рисуется сверху; здесь у её частей появляется
//! высота: пол этажа — плита с картинкой «пол без стен», стены — выдавленные ломаные толщиной
//! из вида стены, над дверью и окном — перемычка, под окном — подоконник со стеклом, створка
//! двери — отдельный узел на петле. Эту сцену пишет выгрузка в `.glb` для 3D Canvas (`glb`),
//! её же рисует тестовая зона — окно 3D в Картографе (`render3d`).
//!
//! Земля этажа ([`Ground`]) — рельеф и твёрдые тела на полу: помосты, ступени, пандусы, марши
//! лестниц между этажами. Пол ложится на рельеф, стены встают на него; верх тел — та же
//! картинка пола, на ней помост и ступени уже нарисованы сверху. Крыши ([`Roof3`]) — скаты с
//! картинкой крыши из 2D, наложенной сверху.
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
/// Выше этого фишка не шагает сама (как в 3D Canvas: треть клетки): стенка помоста, обрыв.
pub const STEP: f32 = 1.0 / 3.0;
/// Стена на рельефе дробится на куски не длиннее этого: низ идёт по земле.
const RELIEF_PIECE: f32 = 1.0;

/// Дерево створок, пока у дверей нет своих моделей.
const WOOD: [u8; 3] = [104, 74, 48];
/// Стекло окна.
const GLASS_TINT: [u8; 3] = [170, 200, 220];
/// Щипцы и торцы крыш.
const GABLE: [u8; 3] = [112, 92, 72];

/// Чем служит узел: по этому выгрузка ставит метки 3D Canvas.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Part {
    Floor,
    Wall,
    Glass,
    Door,
    Roof,
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
    /// Рисунок двери на карте: по нему вид створки в 3D ([`crate::doors3d::look_of`]).
    pub art: Option<String>,
}

/// Верх тела: ровный или плоскостью под уклон (пандус).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Top {
    Flat(f32),
    /// Высота `h` в точке `at` и подъём по осям карты: клеток высоты на клетку пути.
    Slope { at: [f32; 2], h: f32, rise: [f32; 2] },
}

impl Top {
    pub fn at(&self, p: [f32; 2]) -> f32 {
        match *self {
            Top::Flat(h) => h,
            Top::Slope { at, h, rise } => h + (p[0] - at[0]) * rise[0] + (p[1] - at[1]) * rise[1],
        }
    }

    /// Подъём по осям карты (у ровного — ноль).
    fn rise(&self) -> [f32; 2] {
        match *self {
            Top::Flat(_) => [0.0, 0.0],
            Top::Slope { rise, .. } => rise,
        }
    }
}

/// Своя картинка верха тела: `origin` — где у неё левый верхний угол, `u` и `v` — её ширина и
/// высота на карте (векторы, клетки). Марш лестницы между этажами несёт рисунок своего предмета.
#[derive(Clone, Debug, PartialEq)]
pub struct Skin {
    pub picture: Picture,
    pub origin: [f32; 2],
    pub u: [f32; 2],
    pub v: [f32; 2],
}

impl Skin {
    fn uv(&self, p: [f32; 2]) -> [f32; 2] {
        let d = [p[0] - self.origin[0], p[1] - self.origin[1]];
        let along = |a: [f32; 2]| (d[0] * a[0] + d[1] * a[1]) / (a[0] * a[0] + a[1] * a[1]).max(1e-9);
        [along(self.u), along(self.v)]
    }
}

/// Твёрдое тело на полу этажа: помост, ступень, пандус, ступень марша. Бока — отвесно от
/// `base` до верха; верх — картинка пола этажа (помост и ступени на ней уже нарисованы сверху)
/// или своя (`skin`). Бока — цветом `side` или картинкой `face`: по ширине — повтор через
/// [`PANEL`] клетки пути вдоль края, по высоте — от верха до низа один раз.
#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    pub poly: Vec<[f32; 2]>,
    pub base: f32,
    pub top: Top,
    pub side: [u8; 3],
    pub skin: Option<Skin>,
    pub face: Option<Picture>,
}

/// Шаг панелей на боку помоста, клетки пути вдоль края: те же швы, что на 2D-карте.
pub const PANEL: f32 = 0.62;

/// Ключ картинки бока помоста цвета `c` ([`panel_side`]).
pub fn panel_key(c: [u8; 3]) -> String {
    format!("panel_side#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

/// Бок помоста — как его стенка на 2D-карте: от светлого верха к низу темнее, светлая кромка
/// сверху, тёмная линия у пола, шов панели у левого края. Одна панель ([`PANEL`] клетки).
pub fn panel_side(c: [u8; 3]) -> image::RgbaImage {
    let (w, h) = (64u32, 128u32);
    let tone = |k: f32| c.map(|v| (v as f32 * k).round().clamp(0.0, 255.0) as u8);
    image::RgbaImage::from_fn(w, h, |x, y| {
        let f = y as f32 / (h - 1) as f32;
        let rgb = if y < 3 {
            [236, 240, 244]
        } else if y >= h - 4 {
            [24, 28, 34]
        } else if x < 2 {
            tone(0.42)
        } else {
            tone(1.0 - 0.55 * f)
        };
        image::Rgba([rgb[0], rgb[1], rgb[2], 255])
    })
}

/// Высоты земли по сетке: `w × h` точек через `step` клеток, первая — в левом верхнем углу карты.
#[derive(Clone, Debug, PartialEq)]
pub struct Field {
    pub w: usize,
    pub h: usize,
    pub step: f32,
    pub data: Vec<f32>,
}

impl Field {
    fn get(&self, i: isize, j: isize) -> f32 {
        let i = i.clamp(0, self.w as isize - 1) as usize;
        let j = j.clamp(0, self.h as isize - 1) as usize;
        self.data[j * self.w + i]
    }

    /// Высота в точке карты; между точками сетки — по прямой.
    pub fn at(&self, p: [f32; 2]) -> f32 {
        let (x, y) = (p[0] / self.step, p[1] / self.step);
        let (i, j) = (x.floor(), y.floor());
        let (tx, ty) = (x - i, y - j);
        let (i, j) = (i as isize, j as isize);
        let top = self.get(i, j) * (1.0 - tx) + self.get(i + 1, j) * tx;
        let low = self.get(i, j + 1) * (1.0 - tx) + self.get(i + 1, j + 1) * tx;
        top * (1.0 - ty) + low * ty
    }
}

/// Земля этажа над его полом: рельеф и тела на нём. `cliff` — картинка круч рельефа (середина
/// полосы кромки обрыва, как у боков стен); нет — кручи в картинке пола.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Ground {
    pub relief: Option<Field>,
    pub blocks: Vec<Block>,
    pub cliff: Option<Picture>,
}

impl Ground {
    fn relief_at(&self, p: [f32; 2]) -> f32 {
        self.relief.as_ref().map_or(0.0, |f| f.at(p))
    }

    /// Высота земли в точке: верх всего, что там стоит.
    pub fn at(&self, p: [f32; 2]) -> f32 {
        self.blocks.iter().filter(|b| crate::platform::contains(&b.poly, p)).fold(self.relief_at(p), |h, b| h.max(b.top.at(p)))
    }

    /// Куда встанет нога, шагнув в `p` со ступни на высоте `feet`: на самый высокий верх не
    /// выше шага ([`STEP`]); `None` — там тело или земля выше шага (стенка помоста, обрыв).
    pub fn step(&self, p: [f32; 2], feet: f32) -> Option<f32> {
        let reach = feet + STEP + 1e-3;
        let mut h = self.relief_at(p);
        if h > reach {
            return None;
        }
        for b in self.blocks.iter().filter(|b| crate::platform::contains(&b.poly, p)) {
            let t = b.top.at(p);
            if t > reach {
                return None;
            }
            h = h.max(t);
        }
        Some(h)
    }
}

/// Где у этажа есть пол, а где дыра (видно этаж ниже): по точке на `1 / per` клетки.
#[derive(Clone, Debug, PartialEq)]
pub struct Cover {
    pub w: usize,
    pub h: usize,
    pub per: f32,
    pub data: Vec<bool>,
}

impl Cover {
    /// По прозрачности картинки пола во всю карту `cells`: пол — где она непрозрачна.
    pub fn from_alpha(img: &image::RgbaImage, cells: [f32; 2], per: f32) -> Self {
        let (w, h) = ((cells[0] * per).ceil().max(1.0) as usize, (cells[1] * per).ceil().max(1.0) as usize);
        let (kx, ky) = (img.width() as f32 / (cells[0] * per).max(1e-6), img.height() as f32 / (cells[1] * per).max(1e-6));
        let data = (0..w * h)
            .map(|k| {
                let (i, j) = (k % w, k / w);
                let x = (((i as f32 + 0.5) * kx) as u32).min(img.width().saturating_sub(1));
                let y = (((j as f32 + 0.5) * ky) as u32).min(img.height().saturating_sub(1));
                img.get_pixel(x, y)[3] >= 128
            })
            .collect();
        Self { w, h, per, data }
    }

    /// Пол во всю карту `cells` (этаж без дыр).
    pub fn full(cells: [f32; 2], per: f32) -> Self {
        let (w, h) = ((cells[0] * per).ceil().max(1.0) as usize, (cells[1] * per).ceil().max(1.0) as usize);
        Self { w, h, per, data: vec![true; w * h] }
    }

    /// Есть ли пол в точке карты (за краем — нет).
    pub fn has(&self, p: [f32; 2]) -> bool {
        let (i, j) = ((p[0] * self.per).floor(), (p[1] * self.per).floor());
        if i < 0.0 || j < 0.0 || i as usize >= self.w || j as usize >= self.h {
            return false;
        }
        self.data[j as usize * self.w + i as usize]
    }
}

/// Крыша: скаты и щипцы в клетках сцены, высота — над полом этажа. Картинка крыши из 2D
/// ложится на скаты сверху по рамке `frame` (левый верхний угол, ширина, высота — клетки):
/// uv у скатов уже по ней. `outline` — её контур на карте (кольца): под ним фишка — крыша
/// прячется, как тайл крыши в Foundry.
#[derive(Clone, Debug)]
pub struct Roof3 {
    pub picture: Picture,
    pub frame: [f32; 4],
    pub cover: Mesh,
    pub gables: Mesh,
    pub outline: Vec<Vec<[f32; 2]>>,
}

/// Точка внутри колец (внешний контур и дворы: внутри — в нечётном числе колец).
pub fn inside(rings: &[Vec<[f32; 2]>], p: [f32; 2]) -> bool {
    rings.iter().filter(|r| r.len() >= 3 && crate::platform::contains(r, p)).count() % 2 == 1
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
    /// Рельеф и тела на полу.
    pub ground: Ground,
    /// Крыши этажа. Выгрузка кладёт их не в общую модель, а каждую в свой тайл крыши.
    pub roofs: Vec<Roof3>,
    /// Где у этажа пол (выше первого): плита пола — только там, дыры в ней настоящие.
    pub cover: Option<Cover>,
    /// Где над этажом крыши ([`Cap`]): стены под ними — с ровным верхом на высоте свеса.
    pub caps: Vec<Cap>,
}

/// Крыша над стенами этажа: её контур сверху (клетки) и высота свеса над полом этажа
/// ([`eave_over`]). Стена под ней кончается ровно на этой высоте, а не по земле: на рельефе
/// иначе стена со стороны подъёма проходила бы сквозь скат, а с другой не доставала бы до него.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Cap {
    pub poly: Vec<[f32; 2]>,
    pub eave: f32,
}

impl Cap {
    /// Точка под крышей (с запасом на толщину стены по её краю).
    fn covers(&self, p: [f32; 2]) -> bool {
        crate::platform::contains(&self.poly, p) || self.poly.iter().zip(self.poly.iter().cycle().skip(1)).any(|(&a, &b)| seg_dist(p, a, b) < CAP_EDGE)
    }
}

/// Стена у самого края крыши (без свеса) — тоже под ней.
const CAP_EDGE: f32 = 0.2;

fn seg_dist(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let l2 = dx * dx + dy * dy;
    let t = if l2 < 1e-12 { 0.0 } else { (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / l2).clamp(0.0, 1.0) };
    dist(p, [a[0] + dx * t, a[1] + dy * t])
}

/// Высота свеса крыши с контуром `poly` над полом этажа: верх стен ([`STOREY`]) над самой высокой
/// точкой рельефа под ней — ни одна стена под крышей не выйдет сквозь скат.
pub fn eave_over(ground: &Ground, poly: &[[f32; 2]]) -> f32 {
    let Some(f) = ground.relief.as_ref().filter(|_| !poly.is_empty()) else { return STOREY };
    let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
    for p in poly {
        lo = [lo[0].min(p[0]), lo[1].min(p[1])];
        hi = [hi[0].max(p[0]), hi[1].max(p[1])];
    }
    let mut top = poly.iter().map(|&p| f.at(p)).fold(f32::MIN, f32::max);
    let step = 0.25;
    let (nx, ny) = (((hi[0] - lo[0]) / step).ceil() as usize, ((hi[1] - lo[1]) / step).ceil() as usize);
    for j in 0..=ny.min(4000) {
        for i in 0..=nx.min(4000) {
            let p = [(lo[0] + i as f32 * step).min(hi[0]), (lo[1] + j as f32 * step).min(hi[1])];
            if crate::platform::contains(poly, p) {
                top = top.max(f.at(p));
            }
        }
    }
    STOREY + top
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
        // На рельефе пол — сетка высот (кручи — своей картинкой), выше первого этажа — плита
        // там, где пол есть, иначе плита во всю карту.
        let mut prims = Vec::new();
        match (&st.ground.relief, st.cover.as_ref().filter(|_| k > 0)) {
            (Some(f), _) => {
                let cliff = st.ground.cliff.as_ref().map(|p| {
                    let pic = s.picture(p);
                    let mat = s.material(Material { look: Look::Picture(pic), alpha: Alpha::Opaque, two_sided: true, repeat: true, opacity: 1.0 });
                    (mat, p.size[0] as f32 / p.size[1].max(1) as f32)
                });
                let (gentle, steep) = field_mesh(f, size, y0, cliff.map(|c| c.1));
                prims.push((floor_mat, gentle));
                if !steep.idx.is_empty() {
                    prims.push((cliff.map_or(floor_mat, |c| c.0), steep));
                }
            }
            (None, Some(c)) => prims.push((floor_mat, holed_mesh(c, size, y0))),
            (None, None) => {
                let mut m = Mesh::default();
                let (w, h) = (size[0], size[1]);
                quad(&mut m, [[0.0, y0, 0.0], [w, y0, 0.0], [w, y0, h], [0.0, y0, h]], [0.0, 1.0, 0.0], [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]);
                prims.push((floor_mat, m));
            }
        }
        s.nodes.push(Node { name: format!("floor_{}", k + 1), part: Part::Floor, storey: k, at: [0.0; 3], yaw: 0.0, prims, door: None });
        blocks_node(&mut s, k, &st.ground.blocks, size, y0, floor_mat);
        // Низ стен и створок — по земле: на рельефе стена стоит на склоне.
        let relief = st.ground.relief.as_ref();
        let base = |p: [f32; 2]| relief.map_or(0.0, |f| f.at(p));
        let fine = |line: &[[f32; 2]]| if relief.is_some() { subdivide(line, RELIEF_PIECE) } else { line.to_vec() };

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
        // Верх стен под крышей — ровно на её свесе (над полом этажа), прочих — по земле.
        let cap = |p: [f32; 2]| st.caps.iter().filter(|c| c.covers(p)).map(|c| y0 + c.eave).reduce(f32::max);
        let top = WallTop { y: y1, cap: &cap };
        let mut walls = Shell::default();
        for line in &st.walls {
            extrude(&mut walls, &fine(line), half, y0, top, tiling, &base);
        }
        for line in &st.low {
            extrude(&mut walls, &fine(line), half, y0, (y0 + LOW).into(), tiling, &base);
        }
        for g in &st.gaps {
            if dist(g.0, g.1) <= LINTEL_MAX {
                extrude(&mut walls, &fine(&[g.0, g.1]), half, y0 + DOOR, top, tiling, &base);
            }
        }

        let wood = s.material(Material { look: Look::Color(WOOD), alpha: Alpha::Opaque, two_sided: false, repeat: true, opacity: 1.0 });
        let glass = s.material(Material { look: Look::Color(GLASS_TINT), alpha: Alpha::Blend, two_sided: true, repeat: true, opacity: 0.3 });
        for (i, o) in st.openings.iter().enumerate() {
            let (a, b) = o.seg;
            // Створка — на земле у петли.
            let foot = |p: [f32; 2]| y0 + base(p);
            match o.kind {
                PortalKind::Door => {
                    extrude(&mut walls, &fine(&[a, b]), half, y0 + DOOR, top, tiling, &base);
                    let look = Some(crate::doors3d::look_of(o.art.as_deref()));
                    let node = leaf(&mut s, format!("door_{}_{}", k + 1, i + 1), k, (a, b), foot(a), DOOR, (wood, look), Door { id: o.id.clone(), swing: SWING });
                    s.nodes.push(node);
                }
                PortalKind::DoubleDoor => {
                    extrude(&mut walls, &fine(&[a, b]), half, y0 + DOOR, top, tiling, &base);
                    let mid = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
                    let look = Some(crate::doors3d::look_of(o.art.as_deref()));
                    let node = leaf(&mut s, format!("door_{}_{}a", k + 1, i + 1), k, (a, mid), foot(a), DOOR, (wood, look), Door { id: format!("{}_a", o.id), swing: SWING });
                    s.nodes.push(node);
                    let node = leaf(&mut s, format!("door_{}_{}b", k + 1, i + 1), k, (b, mid), foot(b), DOOR, (wood, look), Door { id: format!("{}_b", o.id), swing: -SWING });
                    s.nodes.push(node);
                }
                // Тайный ход снаружи неотличим от стены: створка во всю высоту, из стены.
                PortalKind::Secret => {
                    let node = leaf(&mut s, format!("secret_{}_{}", k + 1, i + 1), k, (a, b), foot(a), STOREY, (side_mat, None), Door { id: o.id.clone(), swing: SWING });
                    s.nodes.push(node);
                }
                PortalKind::Window => {
                    extrude(&mut walls, &[a, b], half, y0, (y0 + SILL).into(), tiling, &base);
                    extrude(&mut walls, &fine(&[a, b]), half, y0 + DOOR, top, tiling, &base);
                    let mut pane = Shell::default();
                    extrude(&mut pane, &[a, b], GLASS / 2.0, y0 + SILL, (y0 + DOOR).into(), tiling, &base);
                    s.nodes.push(Node { name: format!("window_{}_{}", k + 1, i + 1), part: Part::Glass, storey: k, at: [0.0; 3], yaw: 0.0, prims: vec![(glass, pane.into_one())], door: None });
                }
                PortalKind::Opening => {
                    if dist(a, b) <= LINTEL_MAX {
                        extrude(&mut walls, &fine(&[a, b]), half, y0 + DOOR, top, tiling, &base);
                    }
                }
            }
        }
        let prims = if side_mat == wall_mat { vec![(wall_mat, walls.into_one())] } else { vec![(wall_mat, walls.top), (side_mat, walls.side)] };
        let prims: Vec<(usize, Mesh)> = prims.into_iter().filter(|(_, m)| !m.idx.is_empty()).collect();
        if !prims.is_empty() {
            s.nodes.push(Node { name: format!("walls_{}", k + 1), part: Part::Wall, storey: k, at: [0.0; 3], yaw: 0.0, prims, door: None });
        }
        for (i, r) in st.roofs.iter().enumerate() {
            let node = roof_node(&mut s, k, i, r, y0);
            s.nodes.push(node);
        }
    }
    s
}

/// Узел крыши: скаты с её картинкой (прозрачное вне крыши — вырезом), щипцы — цветом. Обе
/// стороны: из-под свеса и из комнаты видна изнанка.
fn roof_node(s: &mut Scene3d, k: usize, i: usize, r: &Roof3, y0: f32) -> Node {
    let pic = s.picture(&r.picture);
    let cover = s.material(Material { look: Look::Picture(pic), alpha: if r.picture.alpha { Alpha::Mask } else { Alpha::Opaque }, two_sided: true, repeat: false, opacity: 1.0 });
    let gable = s.material(Material { look: Look::Color(GABLE), alpha: Alpha::Opaque, two_sided: true, repeat: true, opacity: 1.0 });
    let prims = [(cover, r.cover.clone()), (gable, r.gables.clone())].into_iter().filter(|(_, m)| !m.idx.is_empty()).collect();
    Node { name: format!("roof_{}_{}", k + 1, i + 1), part: Part::Roof, storey: k, at: [0.0, y0, 0.0], yaw: 0.0, prims, door: None }
}

/// Крыша для своего тайла в выгрузке: сцена из одного узла в рамке картинки крыши (середина
/// рамки — в нуле, низ крыши — на нуле) и рамка самой крыши в клетках над полом этажа. 3D Canvas
/// растягивает модель по тайлу картинки, поэтому по углам рамки — по точке без площади: рамка
/// модели равна рамке картинки, даже где крыша её не заполняет.
pub fn roof_scene(r: &Roof3) -> Option<(Scene3d, [[f32; 3]; 2])> {
    let mut s = Scene3d::default();
    let node = roof_node(&mut s, 0, 0, r, 0.0);
    s.nodes.push(node);
    let [lo, hi] = s.bounds()?;
    let [fx, fz, fw, fh] = r.frame;
    let (cx, cz) = (fx + fw / 2.0, fz + fh / 2.0);
    let n = &mut s.nodes[0];
    for (_, m) in &mut n.prims {
        for v in &mut m.v {
            v.p = [v.p[0] - cx, v.p[1] - lo[1], v.p[2] - cz];
        }
    }
    if let Some((_, m)) = n.prims.first_mut() {
        let b = m.v.len() as u32;
        for (x, z) in [(fx, fz), (fx + fw, fz + fh)] {
            m.v.push(Vertex { p: [x - cx, 0.0, z - cz], n: [0.0, 1.0, 0.0], uv: [0.0, 0.0] });
        }
        m.idx.extend([b, b, b + 1]);
    }
    Some((s, [lo, hi]))
}

/// Круча рельефа: треугольник сетки высот круче этого (косинус наклона) — бок обрыва.
const STEEP: f32 = 0.6;
/// Картинка кручи повторяется по высоте через столько клеток.
const CLIFF_PERIOD: f32 = 1.0;

/// Пол на рельефе: сетка высот поля во всю карту, картинка пола — по рамке карты. С картинкой
/// круч (`cliff` — отношение её сторон) кручи отдельно: по ширине — вдоль склона, по высоте —
/// по высоте, а не растянутый сверху пол.
fn field_mesh(f: &Field, size: [f32; 2], y0: f32, cliff: Option<f32>) -> (Mesh, Mesh) {
    let mut m = Mesh::default();
    let mut steep = Mesh::default();
    let h = |i: isize, j: isize| f.get(i, j);
    for j in 0..f.h {
        for i in 0..f.w {
            let (x, z) = ((i as f32 * f.step).min(size[0]), (j as f32 * f.step).min(size[1]));
            let (ii, jj) = (i as isize, j as isize);
            let n = norm3([-(h(ii + 1, jj) - h(ii - 1, jj)) / (2.0 * f.step), 1.0, -(h(ii, jj + 1) - h(ii, jj - 1)) / (2.0 * f.step)]);
            m.v.push(Vertex { p: [x, y0 + h(ii, jj), z], n, uv: [x / size[0].max(1e-6), z / size[1].max(1e-6)] });
        }
    }
    for j in 0..f.h.saturating_sub(1) {
        for i in 0..f.w.saturating_sub(1) {
            let a = (j * f.w + i) as u32;
            let (b, c, d) = (a + 1, a + 1 + f.w as u32, a + f.w as u32);
            // Лицом вверх: x — вправо, z — на юг.
            for t in [[a, d, c], [a, c, b]] {
                let p = t.map(|i| m.v[i as usize].p);
                let e1 = [p[1][0] - p[0][0], p[1][1] - p[0][1], p[1][2] - p[0][2]];
                let e2 = [p[2][0] - p[0][0], p[2][1] - p[0][1], p[2][2] - p[0][2]];
                let n = norm3([e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]]);
                let n = if n[1] < 0.0 { n.map(|v| -v) } else { n };
                match cliff.filter(|_| n[1] < STEEP) {
                    // Круча: по ширине — по той оси карты, вдоль которой склон тянется.
                    Some(aspect) => {
                        let along = |q: [f32; 3]| if n[0].abs() > n[2].abs() { q[2] } else { q[0] };
                        let uv = p.map(|q| [along(q) / (CLIFF_PERIOD * aspect), -(q[1] - y0) / CLIFF_PERIOD]);
                        tri(&mut steep, p, n, uv);
                    }
                    None => m.idx.extend(t),
                }
            }
        }
    }
    (m, steep)
}

/// Пол верхнего этажа там, где он есть (`cover`), прямоугольниками: дыры — настоящие, лучи 3D
/// Canvas (ход фишки по маршу, взгляд вниз) проходят в них, а не упираются в плиту. Пол раздут
/// на точку карты пола: край дыры в картинке — вырез по прозрачности, щели у края нет.
fn holed_mesh(c: &Cover, size: [f32; 2], y0: f32) -> Mesh {
    let (w, h) = (c.w, c.h);
    let at = |i: isize, j: isize| i >= 0 && j >= 0 && (i as usize) < w && (j as usize) < h && c.data[j as usize * w + i as usize];
    let solid: Vec<bool> = (0..w * h)
        .map(|k| {
            let (i, j) = ((k % w) as isize, (k / w) as isize);
            at(i, j) || at(i - 1, j) || at(i + 1, j) || at(i, j - 1) || at(i, j + 1)
        })
        .collect();
    let cell = 1.0 / c.per;
    let mut m = Mesh::default();
    let mut emit = |i0: usize, i1: usize, j0: usize, j1: usize| {
        let (x0, x1) = (i0 as f32 * cell, (i1 as f32 * cell).min(size[0]));
        let (z0, z1) = (j0 as f32 * cell, (j1 as f32 * cell).min(size[1]));
        let uv = |x: f32, z: f32| [x / size[0].max(1e-6), z / size[1].max(1e-6)];
        quad(&mut m, [[x0, y0, z0], [x1, y0, z0], [x1, y0, z1], [x0, y0, z1]], [0.0, 1.0, 0.0], [uv(x0, z0), uv(x1, z0), uv(x1, z1), uv(x0, z1)]);
    };
    // Отрезки пола по строкам; одинаковые отрезки соседних строк — один прямоугольник.
    let mut open: Vec<(usize, usize, usize)> = Vec::new();
    for j in 0..=h {
        let mut runs = Vec::new();
        if j < h {
            let mut i = 0;
            while i < w {
                if solid[j * w + i] {
                    let i0 = i;
                    while i < w && solid[j * w + i] {
                        i += 1;
                    }
                    runs.push((i0, i));
                } else {
                    i += 1;
                }
            }
        }
        let mut next = Vec::new();
        for (i0, i1, j0) in open.drain(..) {
            if runs.contains(&(i0, i1)) {
                next.push((i0, i1, j0));
            } else {
                emit(i0, i1, j0, j);
            }
        }
        for (i0, i1) in runs {
            if !next.iter().any(|o| (o.0, o.1) == (i0, i1)) {
                next.push((i0, i1, j));
            }
        }
        open = next;
    }
    m
}

/// Тела на полу этажа одним узлом: верх — картинкой пола (или своей), бока — цветом тела.
fn blocks_node(s: &mut Scene3d, k: usize, blocks: &[Block], size: [f32; 2], y0: f32, floor_mat: usize) {
    let mut by_mat: Vec<(usize, Mesh)> = Vec::new();
    let put = |mat: usize, by_mat: &mut Vec<(usize, Mesh)>| -> usize {
        match by_mat.iter().position(|(m, _)| *m == mat) {
            Some(i) => i,
            None => {
                by_mat.push((mat, Mesh::default()));
                by_mat.len() - 1
            }
        }
    };
    for b in blocks {
        let poly = crate::platform::tidy(&b.poly);
        if poly.len() < 3 {
            continue;
        }
        let top_mat = match &b.skin {
            Some(sk) => {
                let pic = s.picture(&sk.picture);
                s.material(Material { look: Look::Picture(pic), alpha: if sk.picture.alpha { Alpha::Mask } else { Alpha::Opaque }, two_sided: false, repeat: false, opacity: 1.0 })
            }
            None => floor_mat,
        };
        let side_mat = match &b.face {
            Some(p) => {
                let pic = s.picture(p);
                s.material(Material { look: Look::Picture(pic), alpha: Alpha::Opaque, two_sided: false, repeat: true, opacity: 1.0 })
            }
            None => s.material(Material { look: Look::Color(b.side), alpha: Alpha::Opaque, two_sided: false, repeat: true, opacity: 1.0 }),
        };
        let uv = |p: [f32; 2]| match &b.skin {
            Some(sk) => sk.uv(p),
            None => [p[0] / size[0].max(1e-6), p[1] / size[1].max(1e-6)],
        };
        let rise = b.top.rise();
        let up = norm3([-rise[0], 1.0, -rise[1]]);
        let at = |p: [f32; 2], y: f32| [p[0], y0 + y, p[1]];
        let t = put(top_mat, &mut by_mat);
        for [i, j, l] in crate::platform::triangulate(&poly) {
            let (a, c, d) = (poly[i], poly[j], poly[l]);
            tri(&mut by_mat[t].1, [at(a, b.top.at(a)), at(c, b.top.at(c)), at(d, b.top.at(d))], up, [uv(a), uv(c), uv(d)]);
        }
        let sd = put(side_mat, &mut by_mat);
        let nrm = crate::platform::normals(&poly);
        let mut run = 0.0;
        for e in 0..poly.len() {
            let (a, c) = (poly[e], poly[(e + 1) % poly.len()]);
            let (ha, hc) = (b.top.at(a), b.top.at(c));
            let len = dist(a, c);
            let (u0, u1) = (run, run + len);
            run = u1;
            if ha.max(hc) - b.base < 1e-3 {
                continue;
            }
            let n = [nrm[e][0], 0.0, nrm[e][1]];
            // Картинка бока — панелями вдоль края, по высоте — от верха до низа один раз.
            let (u0, u1, va, vc) = match b.face {
                Some(_) => (u0 / PANEL, u1 / PANEL, 1.0, 1.0),
                None => (u0, u1, ha - b.base, hc - b.base),
            };
            quad(&mut by_mat[sd].1, [at(a, b.base), at(c, b.base), at(c, hc), at(a, ha)], n, [[u0, va], [u1, vc], [u1, 0.0], [u0, 0.0]]);
        }
    }
    let prims: Vec<(usize, Mesh)> = by_mat.into_iter().filter(|(_, m)| !m.idx.is_empty()).collect();
    if !prims.is_empty() {
        s.nodes.push(Node { name: format!("ground_{}", k + 1), part: Part::Floor, storey: k, at: [0.0; 3], yaw: 0.0, prims, door: None });
    }
}

/// Ломаная с кусками не длиннее `piece`.
fn subdivide(line: &[[f32; 2]], piece: f32) -> Vec<[f32; 2]> {
    let mut out = Vec::with_capacity(line.len());
    for (i, &p) in line.iter().enumerate() {
        if i > 0 {
            let q = line[i - 1];
            let n = (dist(q, p) / piece).ceil().max(1.0) as usize;
            for t in 1..n {
                let f = t as f32 / n as f32;
                out.push([q[0] + (p[0] - q[0]) * f, q[1] + (p[1] - q[1]) * f]);
            }
        }
        out.push(p);
    }
    out
}

fn norm3(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-9);
    [v[0] / l, v[1] / l, v[2] / l]
}

/// Треугольник лицом по `n` (как [`quad`]).
pub(crate) fn tri(m: &mut Mesh, p: [[f32; 3]; 3], n: [f32; 3], uv: [[f32; 2]; 3]) {
    let base = m.v.len() as u32;
    for k in 0..3 {
        m.v.push(Vertex { p: p[k], n, uv: uv[k] });
    }
    let e1 = [p[1][0] - p[0][0], p[1][1] - p[0][1], p[1][2] - p[0][2]];
    let e2 = [p[2][0] - p[0][0], p[2][1] - p[0][1], p[2][2] - p[0][2]];
    let c = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
    if c[0] * n[0] + c[1] * n[1] + c[2] * n[2] >= 0.0 {
        m.idx.extend([base, base + 1, base + 2]);
    } else {
        m.idx.extend([base, base + 2, base + 1]);
    }
}

/// Створка от петли `hinge` к `to` на высоте `y0`, высотой `tall`: узел с началом на петле,
/// сетка вдоль его оси x — так 3D Canvas поворачивает её вокруг петли.
#[allow(clippy::too_many_arguments)]
/// Створка от петли `hinge` до `to` высотой `tall` на земле `y0`: модель двери ([`crate::doors3d`])
/// по виду `look`, растянутая по проёму; без вида (тайный ход) или без модели — доска толщиной
/// [`LEAF`] материалом `mat`.
fn leaf(s: &mut Scene3d, name: String, storey: usize, (hinge, to): Segment, y0: f32, tall: f32, (mat, look): (usize, Option<crate::doors3d::DoorLook>), door: Door) -> Node {
    let (dx, dz) = (to[0] - hinge[0], to[1] - hinge[1]);
    let len = dx.hypot(dz).max(0.01);
    if let Some(p) = look.and_then(|l| crate::doors3d::leaf_part(l, len, tall)) {
        let (key, size) = p.picture;
        let pic = s.picture(&Picture { key, size, alpha: false });
        let material = s.material(Material { look: Look::Picture(pic), ..p.material });
        return Node { name, part: Part::Door, storey, at: [hinge[0], y0, hinge[1]], yaw: (-dz).atan2(dx), prims: vec![(material, p.mesh)], door: Some(door) };
    }
    let mut m = Shell::default();
    let tiling = Tiling { top: 1.0, side_u: 1.0, side_v: 1.0 };
    // Чуть короче проёма и ниже притолоки: не трётся о косяк, когда поворачивается.
    extrude(&mut m, &[[0.02, 0.0], [len - 0.02, 0.0]], LEAF / 2.0, 0.0, (tall - 0.02).into(), tiling, &|_| 0.0);
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
/// точка совпадает с последней. `base` поднимает стену в точке оси (земля на рельефе).
/// Верх выдавливаемой стены над полом: `y` (поверх земли в точке) или, где `cap` даёт высоту, —
/// ровно она (стена под крышей).
#[derive(Clone, Copy)]
struct WallTop<'a> {
    y: f32,
    cap: &'a dyn Fn([f32; 2]) -> Option<f32>,
}

fn no_cap(_: [f32; 2]) -> Option<f32> {
    None
}

impl From<f32> for WallTop<'static> {
    fn from(y: f32) -> Self {
        WallTop { y, cap: &no_cap }
    }
}

fn extrude(m: &mut Shell, line: &[[f32; 2]], half: f32, y0: f32, top: WallTop, t: Tiling, base: &dyn Fn([f32; 2]) -> f32) {
    let y1 = top.y;
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
    // Подъём точек оси: обе стороны стены в точке — на одной высоте, верх ровный поперёк.
    let lift: Vec<f32> = pts.iter().map(|&p| base(p)).collect();
    // Верх в точке оси: под крышей — её свес (не ниже низа стены), иначе — над землёй.
    let tops: Vec<Option<f32>> = pts.iter().enumerate().map(|(i, &p)| (top.cap)(p).map(|e| e.max(y0 + lift[i] + 0.05))).collect();
    let at = |p: [f32; 2], i: usize, y: f32| [p[0], if y == y1 && let Some(c) = tops[i] { c } else { y + lift[i] }, p[1]];
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
        quad(&mut m.side, [at(l(i), i, y0), at(l(j), j, y0), at(l(j), j, y1), at(l(i), i, y1)], [nl[0], 0.0, nl[1]], [[su0, sv0], [su1, sv0], [su1, sv1], [su0, sv1]]);
        quad(&mut m.side, [at(r(j), j, y0), at(r(i), i, y0), at(r(i), i, y1), at(r(j), j, y1)], [-nl[0], 0.0, -nl[1]], [[su1, sv0], [su0, sv0], [su0, sv1], [su1, sv1]]);
        let (tu0, tu1) = (u0 / t.top, u1 / t.top);
        quad(&mut m.top, [at(l(i), i, y1), at(l(j), j, y1), at(r(j), j, y1), at(r(i), i, y1)], [0.0, 1.0, 0.0], [[tu0, 0.0], [tu1, 0.0], [tu1, 1.0], [tu0, 1.0]]);
    }
    if !closed {
        let h = (y1 - y0) / t.side_v;
        let w = 2.0 * half / t.side_u;
        let d0 = dir(0, 1);
        quad(&mut m.side, [at(r(0), 0, y0), at(l(0), 0, y0), at(l(0), 0, y1), at(r(0), 0, y1)], [-d0[0], 0.0, -d0[1]], [[0.0, h], [w, h], [w, 0.0], [0.0, 0.0]]);
        let d1 = dir(n - 2, n - 1);
        let e = n - 1;
        quad(&mut m.side, [at(l(e), e, y0), at(r(e), e, y0), at(r(e), e, y1), at(l(e), e, y1)], [d1[0], 0.0, d1[1]], [[0.0, h], [w, h], [w, 0.0], [0.0, 0.0]]);
    }
}

/// Четырёхугольник с нормалью `n`: обход вершин подбирается так, чтобы лицевая сторона
/// смотрела по `n` (glTF: лицо — обход против часовой, если смотреть на него).
pub(crate) fn quad(m: &mut Mesh, p: [[f32; 3]; 4], n: [f32; 3], uv: [[f32; 2]; 4]) {
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
        Storey { floor: None, walls, low: Vec::new(), openings, gaps: Vec::new(), look: plain(0.25), ground: Ground::default(), roofs: Vec::new(), cover: None, caps: Vec::new() }
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
        let door = Opening { seg: ([2.0, 1.0], [3.0, 1.0]), kind: PortalKind::Door, id: "w7".into(), art: None };
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
        let door = Opening { seg: ([1.0, 1.0], [1.0, 2.0]), kind: PortalKind::Door, id: "w1".into(), art: None };
        let s = build([4.0, 4.0], &[storey(Vec::new(), vec![door])]);
        let leaf = s.nodes.iter().find(|n| n.part == Part::Door).unwrap();
        // Створка от петли (1, 1) вниз по карте до (1, 2): после поворота узла она там и лежит.
        assert!((leaf.yaw + std::f32::consts::FRAC_PI_2).abs() < 1e-6);
        let pts = placed(leaf);
        // Толщина — доски створки с железом поверх (петли, кольцо).
        assert!(pts.iter().all(|p| p[2] > 0.95 && p[2] < 2.0 && (p[0] - 1.0).abs() <= 0.1), "{pts:?}");
    }

    #[test]
    fn a_double_door_swings_both_leaves_the_same_way() {
        let door = Opening { seg: ([0.0, 1.0], [2.0, 1.0]), kind: PortalKind::DoubleDoor, id: "w3".into(), art: None };
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
        let win = Opening { seg: ([1.0, 1.0], [2.0, 1.0]), kind: PortalKind::Window, id: "w2".into(), art: None };
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

    /// Луч сверху вниз в точке `p` попадает в сетку.
    fn hits_from_above(m: &Mesh, p: [f32; 2]) -> bool {
        m.idx.chunks(3).any(|t| {
            let q = [0, 1, 2].map(|k| m.v[t[k] as usize].p);
            let s = |a: [f32; 3], b: [f32; 3]| (b[0] - a[0]) * (p[1] - a[2]) - (b[2] - a[2]) * (p[0] - a[0]);
            let (d1, d2, d3) = (s(q[0], q[1]), s(q[1], q[2]), s(q[2], q[0]));
            (d1 >= 0.0 && d2 >= 0.0 && d3 >= 0.0) || (d1 <= 0.0 && d2 <= 0.0 && d3 <= 0.0)
        })
    }

    #[test]
    fn an_upper_floor_has_real_holes_where_its_picture_has_them() {
        // Второй этаж 8×6, дыра 2×3 клетки над маршем: x 3..5, y 1..4.
        let cover = Cover { w: 32, h: 24, per: 4.0, data: (0..32 * 24).map(|k| !((12..20).contains(&(k % 32)) && (4..16).contains(&(k / 32)))).collect() };
        let mut up = storey(Vec::new(), Vec::new());
        up.cover = Some(cover.clone());
        // У первого этажа та же карта пола ничего не режет.
        let mut ground = storey(Vec::new(), Vec::new());
        ground.cover = Some(cover);
        let s = build([8.0, 6.0], &[ground, up]);
        let floor = |name: &str| s.nodes.iter().find(|n| n.name == name).unwrap().prims[0].1.clone();
        let (f1, f2) = (floor("floor_1"), floor("floor_2"));
        assert!(hits_from_above(&f1, [4.0, 2.5]));
        assert!(!hits_from_above(&f2, [4.0, 2.5]), "в дыре луч проходит вниз");
        assert!(hits_from_above(&f2, [1.0, 1.0]) && hits_from_above(&f2, [7.5, 5.5]) && hits_from_above(&f2, [4.0, 5.0]));
        // Край дыры раздут на точку карты пола: у самого края пол есть, глубже — нет.
        assert!(hits_from_above(&f2, [2.9, 2.5]) && !hits_from_above(&f2, [3.4, 2.5]));
        assert!(faces_follow_normals(&f2));
        assert!(f2.v.iter().all(|v| v.p[1] == STOREY && (v.uv[0] - v.p[0] / 8.0).abs() < 1e-6 && (v.uv[1] - v.p[2] / 6.0).abs() < 1e-6));
        // Прямоугольниками, а не по точке: вокруг одной дыры — горстка.
        assert!(f2.idx.len() / 6 <= 4, "{} прямоугольников", f2.idx.len() / 6);
    }

    #[test]
    fn a_cliff_wears_its_own_picture_upright_and_gentle_ground_the_floor() {
        // Ровно, потом обрыв в клетку высотой на полклетки пути, потом ровно.
        let f = Field { w: 9, h: 3, step: 0.5, data: (0..27).map(|k| if k % 9 >= 5 { 1.0 } else { 0.0 }).collect() };
        let mut st = storey(Vec::new(), Vec::new());
        st.ground.relief = Some(f);
        st.ground.cliff = Some(Picture { key: "cliff#side".into(), size: [512, 128], alpha: false });
        let s = build([4.0, 1.0], &[st]);
        let node = s.nodes.iter().find(|n| n.name == "floor_1").unwrap();
        assert_eq!(node.prims.len(), 2);
        let (mat, steep) = &node.prims[1];
        assert!(matches!(s.materials[*mat].look, Look::Picture(i) if s.pictures[i].key == "cliff#side"));
        // Круча — от низа до верха, по высоте картинка на клетку: v от 0 до −1.
        let (lo, hi) = steep.v.iter().fold((f32::MAX, f32::MIN), |(lo, hi), v| (lo.min(v.p[1]), hi.max(v.p[1])));
        assert!(lo.abs() < 1e-6 && (hi - 1.0).abs() < 1e-6);
        assert!(steep.v.iter().all(|v| (v.uv[1] + v.p[1]).abs() < 1e-5));
        assert!(faces_follow_normals(steep));
        // Ровное — картинкой пола; без картинки круч всё — одной сеткой.
        assert!(!node.prims[0].1.idx.is_empty());
        let mut plain = storey(Vec::new(), Vec::new());
        plain.ground.relief = Some(Field { w: 9, h: 3, step: 0.5, data: (0..27).map(|k| if k % 9 >= 5 { 1.0 } else { 0.0 }).collect() });
        assert_eq!(build([4.0, 1.0], &[plain]).nodes.iter().find(|n| n.name == "floor_1").unwrap().prims.len(), 1);
    }

    #[test]
    fn a_dais_side_is_panels_along_the_edge_and_dark_at_the_foot() {
        let mut st = storey(Vec::new(), Vec::new());
        let face = Picture { key: panel_key([98, 106, 116]), size: [64, 128], alpha: false };
        st.ground.blocks.push(Block { poly: vec![[1.0, 1.0], [3.48, 1.0], [3.48, 2.0], [1.0, 2.0]], base: 0.0, top: Top::Flat(0.6), side: [98, 106, 116], skin: None, face: Some(face) });
        let s = build([4.0, 4.0], &[st]);
        let g = s.nodes.iter().find(|n| n.name == "ground_1").unwrap();
        let (mat, sides) = g.prims.iter().find(|(m, _)| matches!(s.materials[*m].look, Look::Picture(i) if s.pictures[i].key.starts_with("panel_side#"))).unwrap();
        assert!(s.materials[*mat].repeat);
        // Северный край 2,48 клетки — четыре панели; низ — v = 1, верх — 0.
        assert!(sides.v.iter().any(|v| (v.uv[0] - 4.0).abs() < 1e-4));
        assert!(sides.v.iter().all(|v| (v.p[1] < 1e-6) == ((v.uv[1] - 1.0).abs() < 1e-6)));
        // Картинка: светлая кромка сверху, тёмная линия у пола, к полу темнее.
        let img = panel_side([98, 106, 116]);
        let luma = |y: u32| img.get_pixel(32, y).0[..3].iter().map(|&c| c as u32).sum::<u32>();
        assert!(luma(1) > luma(10) && luma(10) > luma(100) && luma(100) > luma(126));
    }

    #[test]
    fn the_sides_get_their_own_picture_when_there_is_one() {
        let strip = Picture { key: "stone".into(), size: [1024, 256], alpha: false };
        let side = Picture { key: "stone#side".into(), size: [1024, 128], alpha: false };
        let look = WallLook { width: 0.25, strip: Some(strip), side: Some(side), color: [0; 3] };
        let st = Storey { floor: None, walls: vec![vec![[0.0, 1.0], [2.0, 1.0]]], low: Vec::new(), openings: Vec::new(), gaps: Vec::new(), look, ground: Ground::default(), roofs: Vec::new(), cover: None, caps: Vec::new() };
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
        let st = Storey { floor: None, walls: vec![vec![[0.0, 1.0], [2.0, 1.0]]], low: Vec::new(), openings: Vec::new(), gaps: Vec::new(), look, ground: Ground::default(), roofs: Vec::new(), cover: None, caps: Vec::new() };
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

    #[test]
    fn a_block_stands_on_the_floor_and_wears_the_floor_picture_on_top() {
        let mut st = storey(Vec::new(), Vec::new());
        st.ground.blocks.push(Block { poly: vec![[1.0, 1.0], [3.0, 1.0], [3.0, 2.0], [1.0, 2.0]], base: 0.0, top: Top::Flat(0.6), side: [90, 90, 90], skin: None, face: None });
        let s = build([4.0, 4.0], &[st]);
        let floor_mat = s.nodes.iter().find(|n| n.name == "floor_1").unwrap().prims[0].0;
        let g = s.nodes.iter().find(|n| n.name == "ground_1").unwrap();
        let top = &g.prims.iter().find(|(m, _)| *m == floor_mat).unwrap().1;
        assert!(top.v.iter().all(|v| (v.p[1] - 0.6).abs() < 1e-6 && (v.uv[0] - v.p[0] / 4.0).abs() < 1e-6 && (v.uv[1] - v.p[2] / 4.0).abs() < 1e-6));
        assert!(faces_follow_normals(top));
        let sides = &g.prims.iter().find(|(m, _)| *m != floor_mat).unwrap().1;
        assert_eq!(sides.idx.len(), 4 * 6);
        assert!(faces_follow_normals(sides));
        // Бока смотрят наружу.
        assert!(sides.v.iter().all(|v| (v.p[0] - 2.0) * v.n[0] + (v.p[2] - 1.5) * v.n[2] > 0.0));
    }

    #[test]
    fn on_relief_the_floor_follows_the_ground_and_walls_stand_on_it() {
        // Земля поднимается к востоку на четверть клетки на клетку.
        let f = Field { w: 5, h: 5, step: 1.0, data: (0..25).map(|k| (k % 5) as f32 * 0.25).collect() };
        let mut st = storey(vec![vec![[0.0, 2.0], [4.0, 2.0]]], Vec::new());
        st.ground.relief = Some(f);
        let s = build([4.0, 4.0], &[st]);
        let floor = &s.nodes.iter().find(|n| n.name == "floor_1").unwrap().prims[0].1;
        assert_eq!(floor.v.len(), 25);
        assert!(faces_follow_normals(floor));
        let walls = &s.nodes.iter().find(|n| n.part == Part::Wall).unwrap().prims[0].1;
        let span = |x: f32| walls.v.iter().filter(|v| (v.p[0] - x).abs() < 1e-4).fold((f32::MAX, f32::MIN), |(lo, hi), v| (lo.min(v.p[1]), hi.max(v.p[1])));
        assert_eq!(span(0.0), (0.0, STOREY));
        assert_eq!(span(4.0), (1.0, 1.0 + STOREY));
        assert!((Ground { relief: s_field(), blocks: Vec::new(), cliff: None }.at([2.0, 1.0]) - 0.5).abs() < 1e-6);
    }

    fn s_field() -> Option<Field> {
        Some(Field { w: 5, h: 5, step: 1.0, data: (0..25).map(|k| (k % 5) as f32 * 0.25).collect() })
    }

    #[test]
    fn under_a_roof_on_a_slope_walls_end_level_at_its_eave() {
        // Дом 0..3 по x на склоне к востоку (0,25 на клетку), крыша со свесом 0,25; стена сада — вне.
        let mut st = storey(vec![vec![[0.0, 1.0], [3.0, 1.0], [3.0, 3.0], [0.0, 3.0], [0.0, 1.0]], vec![[3.0, 3.5], [4.0, 3.5]]], Vec::new());
        st.ground.relief = s_field();
        let poly = vec![[-0.25, 0.75], [3.25, 0.75], [3.25, 3.25], [-0.25, 3.25]];
        let eave = eave_over(&st.ground, &poly);
        assert!((eave - (STOREY + 0.8125)).abs() < 1e-4, "свес — над самой высокой землёй под крышей: {eave}");
        st.caps = vec![Cap { poly, eave }];
        let s = build([4.0, 4.0], &[st]);
        let walls = &s.nodes.iter().find(|n| n.part == Part::Wall).unwrap().prims[0].1;
        let tops = |inside: bool| walls.v.iter().filter(|v| v.p[1] > 1.0 && (v.p[2] < 3.3) == inside).map(|v| v.p[1]).fold((f32::MAX, f32::MIN), |(lo, hi), y| (lo.min(y), hi.max(y)));
        let (lo, hi) = tops(true);
        assert!((hi - eave).abs() < 1e-4 && (lo - eave).abs() < 1e-4, "верх дома ровный, на свесе: {lo}..{hi}");
        // Низ — по земле: на западе на нуле, на востоке выше.
        assert!(walls.v.iter().any(|v| v.p[1].abs() < 1e-4) && walls.v.iter().any(|v| (v.p[1] - 0.75).abs() < 1e-4));
        // Стена сада не под крышей — верх по земле.
        let (glo, ghi) = tops(false);
        assert!((glo - (STOREY + 0.75)).abs() < 1e-4 && (ghi - (STOREY + 1.0)).abs() < 1e-4, "{glo}..{ghi}");
        // Без рельефа свес — просто верх стен.
        assert_eq!(eave_over(&Ground::default(), &[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]]), STOREY);
    }

    #[test]
    fn a_roof_for_its_own_tile_fills_the_picture_frame_from_zero_up() {
        let r = crate::map::Roof::new([1.0, 1.0], [5.0, 4.0], crate::map::RoofShape::Gable, "x".into());
        let roof = crate::roof3d::rect(&r, STOREY, 0.0, Picture { key: "roof".into(), size: [10, 10], alpha: true }, [0.5, 0.5, 5.0, 4.0]);
        let (scene, [lo, hi]) = roof_scene(&roof).unwrap();
        let [a, b] = scene.bounds().unwrap();
        assert!((a[0] + 2.5).abs() < 1e-4 && (b[0] - 2.5).abs() < 1e-4 && (a[2] + 2.0).abs() < 1e-4 && (b[2] - 2.0).abs() < 1e-4, "{a:?} {b:?}");
        assert!(a[1].abs() < 1e-6 && (b[1] - (hi[1] - lo[1])).abs() < 1e-5);
        // Свес ниже верха стен, конёк выше.
        assert!(lo[1] < STOREY && hi[1] > STOREY);
    }
}
