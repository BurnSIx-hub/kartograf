//! Местность из OpenStreetMap. Выгрузка `.osm` (на openstreetmap.org — «Экспорт»)
//! становится картой: дома — комнатами по настоящим контурам, улицы, рельсы и
//! каналы — дорогами, парки, вода и платформы — полом, деревья — объектами,
//! названия улиц и больших зданий — подписями для ведущего.
//!
//! - Масштаб D&D: клетка — 5 футов. Участок шире `Options::max_cells` клеток
//!   ужимается — клетка крупнее, об этом предупреждение.
//! - Дом заливается в маску комнат с краем на шаг маски, как у овальной кисти,
//!   поэтому стены встают по контуру под любым углом, а не лесенкой. Соседи
//!   сливаются в одну комнату — общая стена между ними ставится отдельно.
//! - Двери — у входов, отмеченных в OSM (`entrance`), а где их нет — одна, на
//!   стене, ближайшей к улице или дорожке.
//! - Чего сверху не видно, пропускается: тоннели, метро, подземные и внутренние
//!   платформы и переходы (`tunnel`, `layer` и `level` ниже нуля, `indoor`).
//!   Навесы над платформами (`building=roof`) — тоже: под ними должно быть видно рельсы.

use super::*;
use crate::map::segment_distance;
use std::collections::{BTreeSet, HashMap};
use std::f32::consts::{FRAC_PI_4, FRAC_PI_8};

/// Метров в клетке D&D: 5 футов.
pub const CELL_M: f32 = 1.524;

#[derive(Clone, Copy, Debug)]
pub struct Options {
    /// Метров в клетке.
    pub cell_m: f32,
    /// Наибольшая сторона карты в клетках: участок крупнее ужимается.
    pub max_cells: u32,
    /// Досадить рощи на свободную сушу, 0..1: 0 — деревья только по снимку и OSM.
    /// Боевая карта у брода Болтона по снимку вышла голой: берег — трава и галька.
    pub trees: f32,
}

impl Default for Options {
    fn default() -> Self {
        Self { cell_m: CELL_M, max_cells: 256, trees: 0.0 }
    }
}

/// Подсказка со снимка (`tools/osm/cover.py`): что растёт и лежит в каждой точке
/// участка, по рамке выгрузки. В OSM нет одиноких деревьев, орляка по склонам и
/// травы на «голом камне» — снимок их видит. Классы — цветами картинки.
#[derive(Clone, Debug)]
pub struct Cover {
    w: usize,
    h: usize,
    data: Vec<u8>,
    /// Кроны из анализа снимка (`tools/osm/trees.py`, `<имя>.crowns.json`):
    /// место, размер и порода каждого дерева. Нет — ищутся по классам подсказки.
    crowns: Option<Vec<Crown>>,
    /// Сам снимок по той же рамке — сверить покрытие OSM с видом сверху. Есть
    /// только у анализа в программе: у `.cover.png` скриптов снимка рядом нет.
    photo: Option<image::RgbImage>,
}

/// Вид участка на снимке: средняя яркость и насыщенность (размах каналов), 0..255.
#[derive(Clone, Copy, Debug)]
struct Tone {
    value: f32,
    chroma: f32,
}

/// Крона из `<имя>.crowns.json`: центр в долях рамки, радиус в метрах, порода.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct Crown {
    u: f32,
    v: f32,
    r: f32,
    /// `broad`, `conifer` или `bush`.
    #[serde(default)]
    kind: String,
    /// Откуда порода: `nfi:<тип леса>` — решена по карте лесов; `none` — решает
    /// импорт по тегу леса OSM и очереди.
    #[serde(default)]
    src: String,
    /// Место в очереди на хвойную, 0..1: в лесу с долей хвойных `p` хвойные — кроны
    /// с очередью меньше `p` (тёмные и холодные на снимке, куртинами).
    #[serde(default)]
    queue: f32,
}

impl Crown {
    /// Крона из анализа снимка в программе (`place.rs`).
    pub fn new(u: f32, v: f32, r: f32, kind: String, src: String, queue: f32) -> Self {
        Self { u, v, r, kind, src, queue }
    }
}

#[derive(serde::Deserialize)]
struct CrownFile {
    crowns: Vec<Crown>,
}

/// Классы подсказки: открытая трава, кроны (с тенью), бурое (орляк, вереск —
/// тёмное и рыжее), камень (голый камень, галька, песок), вода, сухая трава
/// (скошенная, выгоревшая — светлая и жёлтая: орляк на ней не растёт), голая
/// земля и снег со льдом (их cover.py ставит только вне зелёного биома).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    Grass,
    Trees,
    Brown,
    Rock,
    Water,
    Dry,
    Earth,
    Snow,
    /// Летний лёд ледника: серо-голубое темнее снега.
    Ice,
    /// Моховое верховое болото.
    Bog,
}

impl Cover {
    const COLORS: [([u8; 3], Class); 10] = [
        ([120, 190, 90], Class::Grass),
        ([20, 70, 30], Class::Trees),
        ([170, 90, 50], Class::Brown),
        ([220, 220, 210], Class::Rock),
        ([30, 60, 160], Class::Water),
        ([210, 190, 110], Class::Dry),
        ([150, 120, 85], Class::Earth),
        ([240, 245, 250], Class::Snow),
        ([170, 200, 215], Class::Ice),
        ([110, 120, 70], Class::Bog),
    ];

    /// Из картинки `.cover.png`: каждый пиксель — ближайший цвет класса.
    pub fn from_image(bytes: &[u8]) -> Result<Self, String> {
        let img = image::load_from_memory(bytes).map_err(|e| e.to_string())?.to_rgb8();
        let (w, h) = (img.width() as usize, img.height() as usize);
        let data = img
            .pixels()
            .map(|p| {
                let d = |c: [u8; 3]| (0..3).map(|i| (p.0[i] as i32 - c[i] as i32).pow(2)).sum::<i32>();
                Self::COLORS.iter().enumerate().min_by_key(|(_, (c, _))| d(*c)).map_or(0, |(k, _)| k as u8)
            })
            .collect();
        Ok(Self { w, h, data, crowns: None, photo: None })
    }

    /// Из классов анализа снимка в программе (`place.rs`): номер класса — место
    /// в `COLORS`, по строкам.
    pub fn from_classes(w: usize, h: usize, data: Vec<u8>, crowns: Vec<Crown>) -> Self {
        Self { w, h, data, crowns: Some(crowns), photo: None }
    }

    /// Со снимком, по которому считались классы.
    pub fn with_photo(mut self, photo: image::RgbImage) -> Self {
        self.photo = Some(photo);
        self
    }

    /// Положить рядом с выгрузкой `place.cover.png` и `place.crowns.json` — те же
    /// файлы, что пишут скрипты `tools/osm`: «Местность из OpenStreetMap…» по
    /// `place.osm` из этой папки соберёт ту же карту.
    pub fn save_files(&self, folder: &std::path::Path) -> Result<(), String> {
        let img = image::RgbImage::from_fn(self.w as u32, self.h as u32, |x, y| image::Rgb(Self::COLORS[self.data[y as usize * self.w + x as usize] as usize % Self::COLORS.len()].0));
        img.save(folder.join("place.cover.png")).map_err(|e| e.to_string())?;
        let crowns: Vec<serde_json::Value> = self
            .crowns
            .iter()
            .flatten()
            .map(|c| serde_json::json!({"u": c.u, "v": c.v, "r": c.r, "kind": c.kind, "src": c.src, "queue": c.queue}))
            .collect();
        std::fs::write(folder.join("place.crowns.json"), serde_json::to_vec_pretty(&serde_json::json!({"crowns": crowns})).unwrap_or_default()).map_err(|e| e.to_string())
    }

    /// Доля пикселей того же класса, что у другой подсказки того же размера (сверка).
    pub fn agreement(&self, other: &Cover) -> f32 {
        if (self.w, self.h) != (other.w, other.h) {
            return 0.0;
        }
        self.data.iter().zip(&other.data).filter(|(a, b)| a == b).count() as f32 / self.data.len().max(1) as f32
    }

    /// Породы и источники крон (сверка).
    pub fn crown_kinds(&self) -> Vec<(String, String)> {
        self.crowns.iter().flatten().map(|c| (c.kind.clone(), c.src.clone())).collect()
    }

    /// Доли классов строкой (сверка).
    pub fn class_shares(&self) -> String {
        const NAMES: [&str; 10] = ["grass", "trees", "brown", "rock", "water", "dry", "earth", "snow", "ice", "bog"];
        let n = self.data.len().max(1) as f32;
        (0..10u8)
            .filter_map(|k| {
                let c = self.data.iter().filter(|&&v| v == k).count();
                (c > 0).then(|| format!("{} {:.0}%", NAMES[k as usize], c as f32 / n * 100.0))
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// С кронами из анализа снимка (`<имя>.crowns.json`).
    pub fn with_crowns(mut self, json: &str) -> Result<Self, String> {
        let f: CrownFile = serde_json::from_str(json).map_err(|e| format!("кроны: {e}"))?;
        self.crowns = Some(f.crowns);
        Ok(self)
    }

    /// Ровная подсказка одного класса, для проверок: `classes` по строкам.
    #[cfg(test)]
    fn grid(w: usize, h: usize, at: impl Fn(usize, usize) -> Class) -> Self {
        let data = (0..w * h).map(|k| Self::COLORS.iter().position(|c| c.1 == at(k % w, k / w)).unwrap_or(0) as u8).collect();
        Self { w, h, data, crowns: None, photo: None }
    }

    /// Доля класса в круге радиуса `r` вокруг точки; `u`, `v`, `r` — в долях рамки по x и y.
    fn share(&self, u: f32, v: f32, ru: f32, rv: f32, class: Class) -> f32 {
        self.share_of(u, v, ru, rv, |c| c == class)
    }

    /// Доля классов, что подходят под `pick`, в круге — как [`Cover::share`].
    fn share_of(&self, u: f32, v: f32, ru: f32, rv: f32, pick: impl Fn(Class) -> bool) -> f32 {
        let hit_of: Vec<bool> = Self::COLORS.iter().map(|c| pick(c.1)).collect();
        let (cx, cy) = (u * self.w as f32, v * self.h as f32);
        let (rx, ry) = ((ru * self.w as f32).max(0.5), (rv * self.h as f32).max(0.5));
        let (mut hit, mut all) = (0u32, 0u32);
        for y in (cy - ry).floor() as i64..=(cy + ry).ceil() as i64 {
            for x in (cx - rx).floor() as i64..=(cx + rx).ceil() as i64 {
                let (dx, dy) = ((x as f32 + 0.5 - cx) / rx, (y as f32 + 0.5 - cy) / ry);
                if dx * dx + dy * dy > 1.0 || x < 0 || y < 0 || x as usize >= self.w || y as usize >= self.h {
                    continue;
                }
                all += 1;
                hit += hit_of.get(self.data[y as usize * self.w + x as usize] as usize).copied().unwrap_or(false) as u32;
            }
        }
        if all == 0 { 0.0 } else { hit as f32 / all as f32 }
    }

    /// Пиксель снимка в точке `u`, `v` (доли рамки): яркость и размах каналов.
    fn tone_at(&self, u: f32, v: f32) -> Option<Tone> {
        let photo = self.photo.as_ref()?;
        let (x, y) = ((u * photo.width() as f32).floor(), (v * photo.height() as f32).floor());
        if x < 0.0 || y < 0.0 || x >= photo.width() as f32 || y >= photo.height() as f32 {
            return None;
        }
        let p = photo.get_pixel(x as u32, y as u32).0;
        let (hi, lo) = (p.iter().max().copied().unwrap_or(0) as f32, p.iter().min().copied().unwrap_or(0) as f32);
        Some(Tone { value: hi, chroma: hi - lo })
    }

    /// Доля класса на суше всего снимка (вода не в счёт).
    fn land_share(&self, class: Class) -> f32 {
        let k = Self::COLORS.iter().position(|c| c.1 == class).unwrap_or(0) as u8;
        let w = Self::COLORS.iter().position(|c| c.1 == Class::Water).unwrap_or(0) as u8;
        let land = self.data.iter().filter(|&&d| d != w).count();
        if land == 0 { 0.0 } else { self.data.iter().filter(|&&d| d == k).count() as f32 / land as f32 }
    }

    /// Основа сельского участка по снимку (биом, cover.py и photo.py): в Англии —
    /// луг; где голого больше 45 % при траве меньше 20 % (пустыня Табернас) —
    /// светлая сухая земля; где сухого и голого больше 45 % при траве меньше 30 %
    /// (пастбище Трухильо) — выгоревшая трава; где снега и льда больше 30 % — снег, а
    /// у ледника OSM (`glacier`) — лёд: летом ледник серый, белый снег на нём
    /// выглядел зимой (Мер-де-Глас).
    fn base_floor(&self) -> &'static str {
        let f = |c| self.land_share(c);
        if f(Class::Snow) + f(Class::Ice) > 0.3 {
            // Серый лёд (Мер-де-Глас) — лёд; белый снег и фирн (Мёнхсйох, хоть и
            // на леднике OSM) — снег.
            if f(Class::Ice) > f(Class::Snow) { "ice_glacier" } else { "snow" }
        } else if f(Class::Bog) + f(Class::Brown) > 0.4 && f(Class::Bog) > 0.2 {
            // Болото по Copernicus и снимку: мох, мочажины — бурое.
            "bog_moss"
        } else if f(Class::Earth) + f(Class::Rock) > 0.45 && f(Class::Grass) < 0.2 {
            "earth_pale"
        } else if f(Class::Dry) + f(Class::Brown) + f(Class::Earth) > 0.45 && f(Class::Grass) < 0.3 {
            "straw_grass"
        } else {
            "meadow"
        }
    }

    /// Класс в точке; `u`, `v` — доли рамки. За рамкой — ничего.
    fn at(&self, u: f32, v: f32) -> Option<Class> {
        let (x, y) = ((u * self.w as f32).floor(), (v * self.h as f32).floor());
        if x < 0.0 || y < 0.0 || x as usize >= self.w || y as usize >= self.h {
            return None;
        }
        Self::COLORS.get(self.data[y as usize * self.w + x as usize] as usize).map(|c| c.1)
    }
}

/// Подсказка, приложенная к карте: точки — в клетках карты.
struct Hint<'a> {
    cover: &'a Cover,
    /// Ширина и высота рамки выгрузки в клетках (карта — с округлением вверх).
    span: [f32; 2],
}

impl Hint<'_> {
    /// Доля класса в круге радиуса `r` клеток вокруг `p`.
    fn share(&self, p: [f32; 2], r: f32, class: Class) -> f32 {
        self.cover.share(p[0] / self.span[0], p[1] / self.span[1], r / self.span[0], r / self.span[1], class)
    }

    fn at(&self, p: [f32; 2]) -> Option<Class> {
        self.cover.at(p[0] / self.span[0], p[1] / self.span[1])
    }

    /// Зелень на снимке в круге радиуса `r` клеток: доля травы, крон, бурого и
    /// сухого среди всего, что не вода (тень дома и кроны снимок путает с водой).
    /// Одна вода — не видно, 1.
    fn green(&self, p: [f32; 2], r: f32) -> f32 {
        let (u, v, ru, rv) = (p[0] / self.span[0], p[1] / self.span[1], r / self.span[0], r / self.span[1]);
        let green = self.cover.share_of(u, v, ru, rv, |c| matches!(c, Class::Grass | Class::Trees | Class::Brown | Class::Dry | Class::Bog));
        let seen = 1.0 - self.cover.share_of(u, v, ru, rv, |c| c == Class::Water);
        if seen < 0.2 { 1.0 } else { (green / seen).min(1.0) }
    }

    /// Средний вид сэмплов маски (`k` — номер сэмпла, вес больше половины) на
    /// снимке; `mw` — ширина маски. Меньше 50 сэмплов на снимке — не судить.
    fn tone(&self, samples: &[(usize, f32)], mw: usize) -> Option<Tone> {
        let res = mask::RES as f32;
        let (mut value, mut chroma, mut n) = (0.0f64, 0.0f64, 0u32);
        for &(k, v) in samples {
            if v < 0.5 {
                continue;
            }
            let p = [((k % mw) as f32 + 0.5) / res, ((k / mw) as f32 + 0.5) / res];
            if let Some(t) = self.cover.tone_at(p[0] / self.span[0], p[1] / self.span[1]) {
                value += t.value as f64;
                chroma += t.chroma as f64;
                n += 1;
            }
        }
        (n >= 50).then(|| Tone { value: (value / n as f64) as f32, chroma: (chroma / n as f64) as f32 })
    }

    /// Средний цвет снимка по сэмплам маски, как у [`Hint::tone`].
    fn colour(&self, samples: &[(usize, f32)], mw: usize) -> Option<[u8; 3]> {
        let photo = self.cover.photo.as_ref()?;
        let res = mask::RES as f32;
        let (mut sum, mut n) = ([0.0f64; 3], 0u32);
        for &(k, v) in samples {
            if v < 0.5 {
                continue;
            }
            let p = [((k % mw) as f32 + 0.5) / res, ((k / mw) as f32 + 0.5) / res];
            let (x, y) = ((p[0] / self.span[0] * photo.width() as f32).floor(), (p[1] / self.span[1] * photo.height() as f32).floor());
            if x < 0.0 || y < 0.0 || x >= photo.width() as f32 || y >= photo.height() as f32 {
                continue;
            }
            let px = photo.get_pixel(x as u32, y as u32).0;
            for c in 0..3 {
                sum[c] += px[c] as f64;
            }
            n += 1;
        }
        (n >= 50).then(|| sum.map(|s| (s / n as f64).round() as u8))
    }

    /// Деревья снимка в клетках карты `w`×`h`: из анализа (`crowns.json`), а без
    /// него — найденные по классам ([`Hint::crowns`]); порода тогда не решена, а
    /// очередь на хвойную — куртинами ~25 м.
    fn trees(&self, w: usize, h: usize, cell_m: f32) -> Vec<Spot> {
        match &self.cover.crowns {
            Some(list) => list
                .iter()
                .map(|c| Spot {
                    q: [c.u * self.span[0], c.v * self.span[1]],
                    r: c.r / cell_m,
                    conifer: (c.src.starts_with("nfi") || c.src.starts_with("dlt")).then(|| c.kind == "conifer"),
                    queue: c.queue,
                })
                .collect(),
            None => self.crowns(w, h).into_iter().map(|(q, r)| Spot { q, r, conifer: None, queue: 0.5 + 0.5 * noise(q, 16.0, 5) }).collect(),
        }
    }

    /// Кроны снимка по одной: центр и радиус в клетках карты `w`×`h`. Центр самой
    /// большой — в самой глубокой точке пятна крон (дальше всех от его края),
    /// следующие — по убыванию глубины, пока центр новой не попадёт в уже
    /// найденную. Круглое пятно даёт одну крону, слипшиеся — по кроне на каждое,
    /// сплошной полог — кроны по 10–17 м вплотную. Прежняя посадка дротиками с
    /// шагом 3,2 клетки ставила на одну крону снимка в 20 м три-пять деревьев.
    fn crowns(&self, w: usize, h: usize) -> Vec<([f32; 2], f32)> {
        const G: usize = 3;
        const BUCKET: f32 = 8.0;
        let (gw, gh) = (w * G, h * G);
        let pos = |k: usize| [((k % gw) as f32 + 0.5) / G as f32, ((k / gw) as f32 + 0.5) / G as f32];
        let trees: Vec<bool> = (0..gw * gh).map(|k| self.at(pos(k)) == Some(Class::Trees)).collect();
        let depth = distance_inside(&trees, gw, gh);
        let mut cand: Vec<(f32, usize)> = depth.iter().enumerate().filter(|(_, d)| **d >= CROWN_MIN * G as f32).map(|(k, d)| (*d / G as f32, k)).collect();
        cand.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
        let mut out: Vec<([f32; 2], f32)> = Vec::new();
        let mut near: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
        let reach = ((2.0 * CROWN_LONE) / BUCKET).ceil() as i32;
        for (inner, k) in cand {
            let c = pos(k);
            let (bx, by) = ((c[0] / BUCKET).floor() as i32, (c[1] / BUCKET).floor() as i32);
            let others = || (-reach..=reach).flat_map(move |dx| (-reach..=reach).map(move |dy| (bx + dx, by + dy)));
            // Центр — не в чужой кроне.
            if others().any(|b| near.get(&b).is_some_and(|v| v.iter().any(|&j| length(out[j].0, c) < out[j].1))) {
                continue;
            }
            // Крона сама по себе или в купе — пятно за ней кончается хотя бы с
            // четверти сторон: на круге в 1,6 её радиуса есть просвет. В сплошном
            // пологе кроны уже, чем глубина пятна. Большая крона у аббатства Болтона,
            // слипшаяся с соседними, при пороге «половина круга» урезалась вдвое.
            let ring = 1.6 * inner + 0.5;
            let around = (0..16).filter(|i| self.at([c[0] + ring * (*i as f32 * FRAC_PI_8).cos(), c[1] + ring * (*i as f32 * FRAC_PI_8).sin()]) == Some(Class::Trees)).count();
            let cap = if inner * CROWN_GROW <= CROWN_LONE && around < 12 {
                CROWN_LONE
            } else {
                CROWN_CANOPY.0 + (CROWN_CANOPY.1 - CROWN_CANOPY.0) * (0.5 + 0.5 * noise(c, 3.0, 31))
            };
            let r = (inner * CROWN_GROW).min(cap);
            // Мелкая крона у края большой — лопасть того же пятна, а не дерево:
            // новая выходит из уже найденных хотя бы на половину радиуса. Без
            // этого большую крону обсаживали кольцом мелких.
            if others().any(|b| near.get(&b).is_some_and(|v| v.iter().any(|&j| length(out[j].0, c) < (CROWN_OVERLAP * (r + out[j].1)).max(out[j].1 + 0.5 * r)))) {
                continue;
            }
            near.entry((bx, by)).or_default().push(out.len());
            out.push((c, r));
        }
        out
    }
}

/// Дерево со снимка: центр и радиус кроны в клетках; хвойное ли (известно по
/// карте лесов) и место в очереди на хвойную, 0..1.
struct Spot {
    q: [f32; 2],
    r: f32,
    conifer: Option<bool>,
    queue: f32,
}

/// Слот дерева со снимка: крона `r` клеток, порода по карте лесов (`conifer`),
/// лес OSM под ним и место в очереди на хвойную. Карта лесов главнее тега OSM.
/// В сухом краю (`arid`) лиственное — каменный дуб, куст — сухой.
fn crown_slot(r: f32, conifer: Option<bool>, wood: Option<&Growth>, queue: f32, arid: bool) -> &'static str {
    let needles = conifer.unwrap_or_else(|| wood.is_some_and(|g| queue < conifer_share(g)));
    // Малая крона, которую анализ сам назвал хвойной (ельник по Copernicus), —
    // молодая ель; тег леса и очередь куст в ель не превращают.
    if r < CROWN_BUSH && conifer == Some(true) {
        conifer_slot(r)
    } else if r < CROWN_BUSH {
        if arid { "matorral" } else { "bush_summer" }
    } else if needles {
        conifer_slot(r)
    } else if arid {
        "encina"
    } else {
        "tree_green"
    }
}

/// Куст и папоротник в сухом краю — сухой куст.
fn arid_slot(slot: &'static str, arid: bool) -> &'static str {
    if arid && matches!(slot, "bush_summer" | "bush" | "fern") { "matorral" } else { slot }
}

/// Слот хвойной кроны радиуса `r` клеток: узкая — ель, широкая — сосна.
fn conifer_slot(r: f32) -> &'static str {
    if r < 2.2 { "fir" } else { "pine" }
}

/// Доля хвойных в лесу OSM по его слотам деревьев (`leaf_type`: хвойный — все,
/// лиственный — ни одного, смешанный и без тега — по слотам).
fn conifer_share(g: &Growth) -> f32 {
    let trees: Vec<&&str> = g.slots.iter().filter(|s| s.starts_with("tree") || **s == "pine").collect();
    if trees.is_empty() { 0.0 } else { trees.iter().filter(|s| ***s == "pine").count() as f32 / trees.len() as f32 }
}

/// Кроны сплошного полога, клетки радиуса: в зрелом лесу кроны 12–18 м.
const CROWN_CANOPY: (f32, f32) = (4.0, 6.0);
/// Самая широкая одиночная крона, клетки радиуса: дуб или бук у аббатства — 20 м.
const CROWN_LONE: f32 = 7.0;
/// Соседние кроны стоят не ближе этой доли суммы радиусов: случайная укладка
/// кругов рыхлее сотовой, и при 0,8 в пологе Болтона оставались просветы.
const CROWN_OVERLAP: f32 = 0.75;
/// Крона на карте шире круга, вписанного в пятно на снимке: пятно не круглое.
const CROWN_GROW: f32 = 1.2;
/// Крона уже — куст, клетки радиуса.
const CROWN_BUSH: f32 = 1.3;
/// Самая узкая крона по снимку, клетки радиуса: cover.py снимает всё уже 3 м.
const CROWN_MIN: f32 = 0.9;

/// Расстояние от каждой точки маски до ближайшей точки вне её, в точках сетки
/// (точный евклидов, Фельценшваб — Хуттенлохер). Край сетки краем не считается.
fn distance_inside(mask: &[bool], w: usize, h: usize) -> Vec<f32> {
    const FAR: f64 = 1e20;
    fn pass(f: &[f64], d: &mut [f64], v: &mut [usize], z: &mut [f64]) {
        let n = f.len();
        let (mut k, sq) = (0usize, |q: usize| (q * q) as f64);
        v[0] = 0;
        z[0] = f64::NEG_INFINITY;
        z[1] = f64::INFINITY;
        for q in 1..n {
            loop {
                let s = ((f[q] + sq(q)) - (f[v[k]] + sq(v[k]))) / (2.0 * (q as f64 - v[k] as f64));
                if s <= z[k] {
                    k -= 1;
                    continue;
                }
                k += 1;
                v[k] = q;
                z[k] = s;
                z[k + 1] = f64::INFINITY;
                break;
            }
        }
        k = 0;
        for (q, out) in d.iter_mut().enumerate() {
            while z[k + 1] < q as f64 {
                k += 1;
            }
            *out = (q as f64 - v[k] as f64).powi(2) + f[v[k]];
        }
    }
    let mut g: Vec<f64> = mask.iter().map(|&m| if m { FAR } else { 0.0 }).collect();
    let n = w.max(h);
    let (mut f, mut d, mut v, mut z) = (vec![0.0; n], vec![0.0; n], vec![0usize; n], vec![0.0; n + 1]);
    for x in 0..w {
        for y in 0..h {
            f[y] = g[y * w + x];
        }
        pass(&f[..h], &mut d[..h], &mut v[..h], &mut z[..h + 1]);
        for y in 0..h {
            g[y * w + x] = d[y];
        }
    }
    for y in 0..h {
        f[..w].copy_from_slice(&g[y * w..(y + 1) * w]);
        pass(&f[..w], &mut d[..w], &mut v[..w], &mut z[..w + 1]);
        g[y * w..(y + 1) * w].copy_from_slice(&d[..w]);
    }
    g.iter().map(|&s| if s >= FAR / 2.0 { f32::INFINITY } else { s.sqrt() as f32 }).collect()
}

/// Дерево или куст слота, чья видимая крона — круг радиуса `r` клеток. У картинок
/// FA крона — 0,7–0,95 холста; видимая рамка шире круга той же площади на 7 %.
fn drop_crown(m: &mut Map, slot: &str, q: [f32; 2], rot: f32, r: f32) {
    let pick = asset(slot);
    let across = pick.visible.map_or(0.85 * pick.size[0].max(pick.size[1]), |[l, t, rr, b]| 0.93 * (rr - l).max(b - t));
    drop_as(m, slot, &pick, q, rot, 2.0 * r / across.max(0.1));
}

type Tags = BTreeMap<String, String>;

struct Node {
    lat: f64,
    lon: f64,
    tags: Tags,
}

struct Way {
    nodes: Vec<i64>,
    tags: Tags,
}

struct Relation {
    /// Пути-участники: номер и роль (`outer`, `inner`).
    ways: Vec<i64>,
    tags: Tags,
}

struct Osm {
    /// Юг, запад, север, восток.
    bounds: [f64; 4],
    nodes: HashMap<i64, Node>,
    ways: BTreeMap<i64, Way>,
    relations: BTreeMap<i64, Relation>,
}

fn tags_of(n: roxmltree::Node) -> Tags {
    n.children().filter(|c| c.has_tag_name("tag")).filter_map(|c| Some((c.attribute("k")?.to_owned(), c.attribute("v")?.to_owned()))).collect()
}

fn number(n: roxmltree::Node, attr: &str) -> Option<f64> {
    n.attribute(attr)?.parse().ok()
}

fn ident(n: roxmltree::Node) -> Option<i64> {
    n.attribute("id")?.parse().ok()
}

fn parse(xml: &str) -> Result<Osm, String> {
    let mut osm = Osm { bounds: [f64::NAN; 4], nodes: HashMap::new(), ways: BTreeMap::new(), relations: BTreeMap::new() };
    let bounds = parse_into(&mut osm, xml)?;
    osm.bounds = match bounds {
        Some([Some(s), Some(w), Some(n), Some(e)]) if n > s && e > w => [s, w, n, e],
        _ => {
            if osm.nodes.is_empty() {
                return Err("в выгрузке нет ни одной точки".into());
            }
            let lat = osm.nodes.values().map(|n| n.lat);
            let lon = osm.nodes.values().map(|n| n.lon);
            [lat.clone().fold(f64::MAX, f64::min), lon.clone().fold(f64::MAX, f64::min), lat.fold(f64::MIN, f64::max), lon.fold(f64::MIN, f64::max)]
        }
    };
    Ok(osm)
}

/// Природный мультиполигон, неполный в выгрузке: «Экспорт» openstreetmap.org
/// отдаёт только пути, что задевают участок, и большой лес, озеро или осыпь
/// не замыкаются — импорт их выбрасывал (осыпь Уэст-Уотер, луг Лулворта).
/// Номера таких отношений — докачать целиком (`relation/{id}/full`).
pub fn incomplete(xml: &str) -> Vec<i64> {
    let Ok(osm) = parse(xml) else { return Vec::new() };
    osm.relations
        .iter()
        .filter(|(_, r)| get(&r.tags, "type") == "multipolygon" && !unseen(&r.tags) && (ground(&r.tags, false).is_some() || growth(&r.tags).is_some()))
        .filter(|(_, r)| r.ways.iter().any(|w| !osm.ways.contains_key(w)))
        .map(|(id, _)| *id)
        .take(MAX_FETCH)
        .collect()
}

/// Сколько неполных контуров докачивать за один импорт.
pub const MAX_FETCH: usize = 25;

/// Узлы, пути и отношения из докачанной выгрузки — к своим; рамка — своя.
fn parse_into(osm: &mut Osm, xml: &str) -> Result<Option<[Option<f64>; 4]>, String> {
    let doc = roxmltree::Document::parse(xml).map_err(|e| format!("не читается как OSM: {e}"))?;
    let root = doc.root_element();
    if root.tag_name().name() != "osm" {
        return Err("это не выгрузка OpenStreetMap (.osm)".into());
    }
    let mut bounds = None;
    for n in root.children().filter(|n| n.is_element()) {
        match n.tag_name().name() {
            "bounds" => bounds = Some([number(n, "minlat"), number(n, "minlon"), number(n, "maxlat"), number(n, "maxlon")]),
            "node" => {
                if let (Some(id), Some(lat), Some(lon)) = (ident(n), number(n, "lat"), number(n, "lon")) {
                    osm.nodes.insert(id, Node { lat, lon, tags: tags_of(n) });
                }
            }
            "way" => {
                if let Some(id) = ident(n) {
                    let nodes = n.children().filter(|c| c.has_tag_name("nd")).filter_map(|c| c.attribute("ref")?.parse().ok()).collect();
                    osm.ways.insert(id, Way { nodes, tags: tags_of(n) });
                }
            }
            "relation" => {
                if let Some(id) = ident(n) {
                    let ways = n
                        .children()
                        .filter(|c| c.has_tag_name("member") && c.attribute("type") == Some("way") && matches!(c.attribute("role"), Some("outer" | "inner" | "")))
                        .filter_map(|c| c.attribute("ref")?.parse().ok())
                        .collect();
                    osm.relations.insert(id, Relation { ways, tags: tags_of(n) });
                }
            }
            _ => {}
        }
    }
    Ok(bounds)
}

fn get<'a>(t: &'a Tags, k: &str) -> &'a str {
    t.get(k).map_or("", String::as_str)
}

/// Под землёй, в тоннеле или внутри здания: на карте сверху этого не видно.
fn unseen(t: &Tags) -> bool {
    let below = |k: &str| get(t, k).split(';').filter_map(|v| v.trim().parse::<f32>().ok()).any(|v| v < 0.0);
    !matches!(get(t, "tunnel"), "" | "no") || below("layer") || below("level") || matches!(get(t, "location"), "underground" | "indoor") || !matches!(get(t, "indoor"), "" | "no")
}

/// Этаж линии для порядка рисования: мост поверх того, что под ним.
fn layer(t: &Tags) -> f32 {
    let l = get(t, "layer").trim().parse::<f32>().unwrap_or(0.0);
    if matches!(get(t, "bridge"), "" | "no") { l } else { l.max(1.0) }
}

/// Ширина из тега `width` в метрах, если она правдоподобна.
fn width_tag(t: &Tags) -> Option<f32> {
    get(t, "width").trim().trim_end_matches('m').trim().parse::<f32>().ok().filter(|w| (0.3..80.0).contains(w))
}

/// Широта и долгота — в клетки карты: равнопромежуточная проекция у середины
/// участка; на паре километров ошибка меньше клетки.
struct Proj {
    north: f64,
    west: f64,
    kx: f64,
    ky: f64,
}

impl Proj {
    fn at(&self, lat: f64, lon: f64) -> [f32; 2] {
        [((lon - self.west) * self.kx) as f32, ((self.north - lat) * self.ky) as f32]
    }
}

/// Замкнутые кольца из кусков: пути мультиполигона цепляются концами.
/// Незамкнувшееся (часть участников за краем выгрузки) отбрасывается.
fn assemble(parts: Vec<Vec<i64>>) -> Vec<Vec<i64>> {
    let mut out = Vec::new();
    let mut open: Vec<Vec<i64>> = Vec::new();
    for w in parts {
        if w.len() >= 4 && w.first() == w.last() {
            out.push(w);
        } else if w.len() >= 2 {
            open.push(w);
        }
    }
    while let Some(mut cur) = open.pop() {
        loop {
            if cur.len() >= 4 && cur.first() == cur.last() {
                out.push(cur);
                break;
            }
            let end = cur[cur.len() - 1];
            let Some(k) = open.iter().position(|w| w[0] == end || w[w.len() - 1] == end) else { break };
            let mut w = open.swap_remove(k);
            if w[0] != end {
                w.reverse();
            }
            cur.extend_from_slice(&w[1..]);
        }
    }
    out
}

/// Площадной объект: кольца в клетках (без повтора первой точки) и узлы колец.
struct Area<'a> {
    rings: Vec<Vec<[f32; 2]>>,
    nodes: Vec<Vec<i64>>,
    tags: &'a Tags,
}

fn areas<'a>(osm: &'a Osm, proj: &Proj) -> Vec<Area<'a>> {
    // Кольца точек и узлов идут парами по номеру: по узлам ищутся общие точки соседей.
    let to_area = |rings: Vec<Vec<i64>>, tags: &'a Tags| {
        let (mut pts, mut ids) = (Vec::new(), Vec::new());
        for r in rings {
            let p: Vec<[f32; 2]> = r[..r.len() - 1].iter().filter_map(|id| osm.nodes.get(id)).map(|n| proj.at(n.lat, n.lon)).collect();
            if p.len() >= 3 {
                pts.push(p);
                ids.push(r);
            }
        }
        (!pts.is_empty()).then_some(Area { rings: pts, nodes: ids, tags })
    };
    let mut out = Vec::new();
    for w in osm.ways.values() {
        if w.nodes.len() >= 4 && w.nodes.first() == w.nodes.last() && !w.tags.is_empty() && !unseen(&w.tags) {
            out.extend(to_area(vec![w.nodes.clone()], &w.tags));
        }
    }
    for r in osm.relations.values() {
        if get(&r.tags, "type") != "multipolygon" || unseen(&r.tags) {
            continue;
        }
        let parts = r.ways.iter().filter_map(|id| osm.ways.get(id)).map(|w| w.nodes.clone()).collect();
        out.extend(to_area(assemble(parts), &r.tags));
    }
    out
}

/// Что за пол у площадного объекта: текстура и порядок слоя (выше — поверх).
/// У леса своего пола нет: подстилка ложится пятнами под деревьями (`plant`).
/// Глубокая вода ложится с мелким краем (`WATER_EDGE`). Пляж без покрытия вдали
/// от моря (`sea` — в выгрузке есть берег моря) — галька: у реки песка почти не бывает.
fn ground(t: &Tags, sea: bool) -> Option<(&'static str, u8)> {
    let (landuse, leisure, natural, amenity) = (get(t, "landuse"), get(t, "leisure"), get(t, "natural"), get(t, "amenity"));
    if matches!(leisure, "swimming_pool") || matches!(amenity, "fountain") {
        return Some(("water_shallow", 4));
    }
    if natural == "water" || matches!(landuse, "basin" | "reservoir") || get(t, "waterway") == "riverbank" {
        return Some(("water_deep", 4));
    }
    if natural == "wetland" {
        let bog = matches!(get(t, "wetland"), "bog" | "string_bog" | "palsa_bog");
        return Some((if bog { "bog_moss" } else { "swamp" }, 3));
    }
    // Ледник — лёд; морена на нём — по снимку (камень).
    if natural == "glacier" {
        return Some(("ice_glacier", 3));
    }
    // Пляж и скала — поверх воды: в OSM пляж лежит по обе стороны линии берега
    // (половина — приливная полоса), причал начинается от его внешнего края, и
    // море, закрывшее эту половину, оставляло причал висеть в воде.
    if matches!(natural, "bare_rock" | "scree" | "stone") {
        return Some((rock_kind(t).0, 5));
    }
    let surface = get(t, "surface");
    if natural == "shingle" || (natural == "beach" && (matches!(surface, "gravel" | "pebblestone" | "shingle" | "stone") || (surface.is_empty() && !sea))) {
        return Some(("shingle", 5));
    }
    if matches!(natural, "sand" | "beach" | "dune") {
        return Some(("sand", 5));
    }
    if matches!(landuse, "grass" | "meadow" | "village_green" | "recreation_ground" | "cemetery" | "allotments" | "flowerbed" | "orchard")
        || matches!(leisure, "park" | "garden" | "pitch" | "playground" | "common" | "dog_park")
        || natural == "grassland"
        || amenity == "grave_yard"
    {
        return Some(("meadow", 2));
    }
    if matches!(natural, "heath" | "scrub") || landuse == "farmland" {
        return Some(("rough_grass", 2));
    }
    if landuse == "farmyard" {
        return Some(("dirt", 2));
    }
    if landuse == "railway" {
        return Some(("gravel", 1));
    }
    if plaza(t) {
        // Мощение площади — по её `surface`: Красная площадь — брусчатка (`sett`),
        // а не плитка, по которой пешеходные линии ложились тёмными полосами.
        let hw = match get(t, "area:highway") {
            "" => "pedestrian",
            hw => hw,
        };
        return Some((paving(t, hw, false), 5));
    }
    if get(t, "man_made") == "pier" {
        return Some(("wood_planks", 6));
    }
    None
}

/// Порода голого камня по тегам `rock`, `geological`, `surface`: слот пола и тон
/// камней FA (`Boulder_Stone_Earthy`, `_Slate`…). Прежний `rocky_ground` брал любую
/// породу на карту, вплоть до красной. Без тега — светлая: на снимке освещённая
/// скала почти всегда светлее травы (мел Лулворта, известняк Малхема), а
/// «землистая» `C` на карте вышла тёмно-серой.
fn rock_kind(t: &Tags) -> (&'static str, &'static str) {
    let v = format!("{};{};{}", get(t, "rock"), get(t, "geological"), get(t, "surface")).to_lowercase();
    let has = |w: &[&str]| w.iter().any(|w| v.contains(w));
    if has(&["chalk", "limestone", "marl", "dolomite", "marble"]) {
        ("rock_chalk", "earthy")
    } else if has(&["red_sandstone", "redrock", "red sandstone"]) {
        ("rock_redrock", "redrock")
    } else if has(&["sandstone", "gritstone"]) {
        ("rock_sandstone", "sandstone")
    } else if has(&["slate", "shale", "schist", "mudstone", "granite", "gneiss"]) {
        ("rock_slate", "slate")
    } else if has(&["basalt", "volcanic", "lava", "tuff", "obsidian", "andesite"]) {
        ("rock_volcanic", "volcanic")
    } else if has(&["earth", "conglomerate", "clay"]) {
        ("rock_earthy", "earthy")
    } else {
        ("rock_chalk", "earthy")
    }
}

/// Сколько клеток у берега глубокого водоёма мелководье.
const WATER_EDGE: f32 = 2.0;

/// За сколько клеток от мелководья море набирает полную глубину.
const SEA_SHELF: f32 = 24.0;

/// Шаг камней-переходов в клетках: человеку — шаг, фигурке — клетка.
const STEPPING: f32 = 0.9;

/// Озёра и реки одним слоем: мелководье по всей воде, глубина — дальше
/// `WATER_EDGE` от берега общей воды. Река из нескольких кусков в OSM иначе
/// получала мели поперёк русла на стыках; край карты берегом не считается —
/// вода уходит за него.
fn water(m: &mut Map, floors: &mut Floors, parts: &[&Area], bog: bool) {
    let (mw, mh) = (m.rooms.w, m.rooms.h);
    // Сумма, а не наибольшее: на общем крае двух кусков каждый даёт половину,
    // и по наибольшему вдоль шва оставалась полоса суши в полводы.
    let mut union = vec![0.0f32; mw * mh];
    for a in parts {
        for (i, v) in coverage(&m.rooms, &a.rings, 0.0) {
            union[i] = (union[i] + v).min(1.0);
        }
    }
    let weights: Vec<(usize, f32)> = union.iter().enumerate().filter(|(_, v)| **v > 0.0).map(|(i, v)| (i, *v)).collect();
    if weights.is_empty() {
        return;
    }
    // Вода FA полупрозрачна: под морем — песчаное дно, иначе мелководье у берега
    // бурое, как трава под водой, а на снимке там бирюза.
    let mut shelf = vec![false; mw * mh];
    let seas: Vec<&&Area> = parts.iter().filter(|a| get(a.tags, "water") == "sea").collect();
    if !seas.is_empty() {
        let sand = floors.get(m, "sand");
        for a in seas {
            let cov = coverage(&m.rooms, &a.rings, 0.0);
            for &(i, v) in &cov {
                shelf[i] |= v >= 0.5;
            }
            m.paint(Layer::Floor, sand, &cov);
        }
    }
    let shallow = floors.get(m, if bog { "bog_water_edge" } else { "water_shallow" });
    // У бочага берег — торфяной обрыв: мелкой полосы почти нет.
    let edge = if bog { 0.6 } else { WATER_EDGE };
    m.paint(Layer::Floor, shallow, &weights);
    // Расстояние до суши по сетке маски, фаской 3–4 (3 — шаг по прямой).
    let far = u32::MAX / 2;
    let mut dist: Vec<u32> = union.iter().map(|&v| if v >= 0.5 { far } else { 0 }).collect();
    let step = |dist: &mut Vec<u32>, i: usize, j: usize, di: isize, dj: isize, cost: u32| {
        let (ni, nj) = (i as isize + di, j as isize + dj);
        if ni >= 0 && nj >= 0 && (ni as usize) < mw && (nj as usize) < mh {
            let n = dist[nj as usize * mw + ni as usize] + cost;
            if n < dist[j * mw + i] {
                dist[j * mw + i] = n;
            }
        }
    };
    for j in 0..mh {
        for i in 0..mw {
            if dist[j * mw + i] > 0 {
                for (di, dj, c) in [(-1, 0, 3), (0, -1, 3), (-1, -1, 4), (1, -1, 4)] {
                    step(&mut dist, i, j, di, dj, c);
                }
            }
        }
    }
    for j in (0..mh).rev() {
        for i in (0..mw).rev() {
            if dist[j * mw + i] > 0 {
                for (di, dj, c) in [(1, 0, 3), (0, 1, 3), (1, 1, 4), (-1, 1, 4)] {
                    step(&mut dist, i, j, di, dj, c);
                }
            }
        }
    }
    let res = mask::RES as f32;
    let aa = 0.5 / res;
    // У моря отмель широкая и глубина набирается плавно — на снимке бирюза у
    // берега переходит в синеву десятками метров; у озера и реки край резкий.
    let deep: Vec<(usize, f32)> = dist
        .iter()
        .enumerate()
        .filter(|(_, d)| **d > 0)
        .map(|(i, &d)| {
            let cells = d.min(far) as f32 / 3.0 / res;
            let v = if shelf[i] { ((cells - edge) / SEA_SHELF).clamp(0.0, 1.0).powf(0.8) } else { 0.5 + (cells - edge) / (2.0 * aa) };
            (i, v.clamp(0.0, 1.0))
        })
        .filter(|(_, v)| *v > 0.0)
        .collect();
    // Глубина моря — бирюзой потемнее, озёр и рек — синей.
    let (sea, inland): (Vec<(usize, f32)>, Vec<(usize, f32)>) = deep.into_iter().partition(|(i, _)| shelf[*i]);
    for (slot, part) in [("sea_deep", sea), (if bog { "bog_water" } else { "water_deep" }, inland)] {
        if !part.is_empty() {
            let t = floors.get(m, slot);
            m.paint(Layer::Floor, t, &part);
        }
    }
}

/// Запас рамки моря за краем карты в клетках: изгиб природного контура рамку
/// внутрь карты не заводит.
const SEA_FRAME: f32 = 3.0;

/// Море — кольца в клетках. Площадью моря в OSM нет: есть линия берега
/// `natural=coastline`, суша слева по ходу, вода справа (на карте, где y вниз, —
/// слева по формуле). Линия режется рамкой карты с запасом, а море замыкается
/// вдоль рамки по часовой стрелке на экране до следующего входа линии в рамку.
/// Острова целиком в рамке — дыры (кольца чётно-нечётно). Конец линии внутри
/// рамки — выгрузка обрезала путь — продлевается по ходу до рамки. Линии в
/// рамке нет — море или суша по стороне ближайшего отрезка от середины карты.
fn sea_rings(osm: &Osm, proj: &Proj, w: f32, h: f32) -> Vec<Vec<[f32; 2]>> {
    let coast: Vec<&Way> = osm.ways.values().filter(|w| get(&w.tags, "natural") == "coastline" && w.nodes.len() >= 2).collect();
    if coast.is_empty() {
        return Vec::new();
    }
    // Цепочки: конец пути — начало следующего; сначала от голов, потом кольца.
    let starts: HashMap<i64, usize> = coast.iter().enumerate().map(|(k, w)| (w.nodes[0], k)).collect();
    let ends: BTreeSet<i64> = coast.iter().filter_map(|w| w.nodes.last().copied()).collect();
    let mut used = vec![false; coast.len()];
    let mut chains: Vec<(Vec<[f32; 2]>, bool)> = Vec::new();
    let heads: Vec<usize> = (0..coast.len()).filter(|&k| !ends.contains(&coast[k].nodes[0])).chain(0..coast.len()).collect();
    for k0 in heads {
        if used[k0] {
            continue;
        }
        used[k0] = true;
        let mut ids = coast[k0].nodes.clone();
        let closed = loop {
            let end = ids[ids.len() - 1];
            if end == ids[0] {
                break true;
            }
            match starts.get(&end) {
                Some(&k) if !used[k] => {
                    used[k] = true;
                    ids.extend_from_slice(&coast[k].nodes[1..]);
                }
                _ => break false,
            }
        };
        let mut pts: Vec<[f32; 2]> = ids.iter().filter_map(|id| osm.nodes.get(id)).map(|n| proj.at(n.lat, n.lon)).collect();
        pts.dedup();
        if closed && pts.len() > 1 && pts.first() == pts.last() {
            pts.pop();
        }
        if pts.len() >= 2 {
            chains.push((pts, closed));
        }
    }
    let m = SEA_FRAME;
    let (x0, y0, x1, y1) = (-m, -m, w + m, h + m);
    let in_frame = |p: [f32; 2]| p[0] > x0 && p[0] < x1 && p[1] > y0 && p[1] < y1;
    let lerp = |a: [f32; 2], b: [f32; 2], t: f32| [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
    // Отрезок против рамки (Лян — Барски): доли входа и выхода.
    let clip = |a: [f32; 2], b: [f32; 2]| -> Option<(f32, f32)> {
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let (mut t0, mut t1) = (0.0f32, 1.0f32);
        for (p, q) in [(-dx, a[0] - x0), (dx, x1 - a[0]), (-dy, a[1] - y0), (dy, y1 - a[1])] {
            if p == 0.0 {
                if q < 0.0 {
                    return None;
                }
                continue;
            }
            let r = q / p;
            if p < 0.0 {
                t0 = t0.max(r);
            } else {
                t1 = t1.min(r);
            }
        }
        (t0 < t1).then_some((t0, t1))
    };
    // Луч из точки внутри рамки до рамки.
    let ray = |p: [f32; 2], d: [f32; 2]| {
        let mut t = f32::INFINITY;
        for (c, lo, hi) in [(0, x0, x1), (1, y0, y1)] {
            if d[c] > 1e-9 {
                t = t.min((hi - p[c]) / d[c]);
            } else if d[c] < -1e-9 {
                t = t.min((lo - p[c]) / d[c]);
            }
        }
        if t.is_finite() { [p[0] + d[0] * t, p[1] + d[1] * t] } else { [x0, p[1]] }
    };
    let mut pieces: Vec<Vec<[f32; 2]>> = Vec::new();
    let mut islands: Vec<Vec<[f32; 2]>> = Vec::new();
    for (pts, closed) in &chains {
        let pts = if *closed {
            // Кольцо — с точки за рамкой, чтобы кусок в рамке не рвался на старте.
            match pts.iter().position(|&p| !in_frame(p)) {
                None => {
                    islands.push(pts.clone());
                    continue;
                }
                Some(k) => pts[k..].iter().chain(&pts[..=k]).copied().collect::<Vec<_>>(),
            }
        } else {
            pts.clone()
        };
        let mut cur: Option<Vec<[f32; 2]>> = in_frame(pts[0]).then(|| vec![ray(pts[0], [pts[0][0] - pts[1][0], pts[0][1] - pts[1][1]]), pts[0]]);
        for s in pts.windows(2) {
            let Some((t0, t1)) = clip(s[0], s[1]) else { continue };
            let c = cur.get_or_insert_with(|| vec![lerp(s[0], s[1], t0)]);
            if t1 < 1.0 {
                c.push(lerp(s[0], s[1], t1));
                pieces.extend(cur.take());
            } else {
                c.push(s[1]);
            }
        }
        if let Some(mut c) = cur {
            let n = c.len();
            c.push(ray(c[n - 1], [c[n - 1][0] - c[n - 2][0], c[n - 1][1] - c[n - 2][1]]));
            pieces.push(c);
        }
    }
    // Место на рамке: путь по часовой стрелке от левого верхнего угла.
    let (fw, fh) = (x1 - x0, y1 - y0);
    let per = 2.0 * (fw + fh);
    let at = |p: [f32; 2]| {
        let d = [(p[1] - y0).abs(), (x1 - p[0]).abs(), (y1 - p[1]).abs(), (p[0] - x0).abs()];
        match (0..4).min_by(|&i, &j| d[i].total_cmp(&d[j])).unwrap_or(0) {
            0 => (p[0] - x0).clamp(0.0, fw),
            1 => fw + (p[1] - y0).clamp(0.0, fh),
            2 => fw + fh + (x1 - p[0]).clamp(0.0, fw),
            _ => 2.0 * fw + fh + (y1 - p[1]).clamp(0.0, fh),
        }
    };
    let corners = [(fw, [x1, y0]), (fw + fh, [x1, y1]), (2.0 * fw + fh, [x0, y1]), (per, [x0, y0])];
    let mut rings = Vec::new();
    if !pieces.is_empty() {
        let entries: Vec<f32> = pieces.iter().map(|p| at(p[0])).collect();
        let exits: Vec<f32> = pieces.iter().map(|p| at(p[p.len() - 1])).collect();
        let mut done = vec![false; pieces.len()];
        for first in 0..pieces.len() {
            if done[first] {
                continue;
            }
            let mut ring = Vec::new();
            let mut k = first;
            for _ in 0..pieces.len() {
                done[k] = true;
                ring.extend_from_slice(&pieces[k]);
                let from = exits[k];
                let ahead = |i: usize| (entries[i] - from).rem_euclid(per);
                let next = (0..pieces.len()).min_by(|&i, &j| ahead(i).total_cmp(&ahead(j))).unwrap_or(first);
                // Углы рамки между выходом и следующим входом.
                let to = from + ahead(next);
                for lap in [0.0, per] {
                    for &(s, c) in &corners {
                        if s + lap > from && s + lap < to {
                            ring.push(c);
                        }
                    }
                }
                if next == first || done[next] {
                    break;
                }
                k = next;
            }
            if ring.len() >= 3 {
                rings.push(ring);
            }
        }
    } else {
        // Ни один кусок не входит в рамку: вся карта — море, если середина со
        // стороны воды (острова — дыры), иначе суша.
        let c = [w / 2.0, h / 2.0];
        let (mut best, mut side) = (f32::INFINITY, 0.0f32);
        for (pts, closed) in &chains {
            let n = pts.len();
            for k in 0..if *closed { n } else { n - 1 } {
                let (a, b) = (pts[k], pts[(k + 1) % n]);
                let d = segment_distance(c, a, b);
                let cross = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
                let len = length(a, b).max(1e-6);
                // У общей вершины двух отрезков решает тот, от чьей прямой точка дальше.
                if d < best - 1e-4 || (d < best + 1e-4 && (cross / len).abs() > side.abs()) {
                    best = best.min(d);
                    side = cross / len;
                }
            }
        }
        if side > 0.0 {
            rings.push(vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]]);
        }
    }
    if !islands.is_empty() && rings.is_empty() {
        rings.push(vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]]);
    }
    rings.extend(islands);
    rings
}

/// Что растёт на площадном объекте: слот, шаг между посадками в клетках, масштаб
/// (от и до) и доля мест, где правда посажено (вереск — редкие кусты).
struct Growth {
    slots: &'static [&'static str],
    spacing: f32,
    scale: (f32, f32),
    share: f64,
    /// Подстилка под кронами — у леса.
    litter: bool,
    /// Старшинство: контуры в OSM перекрываются (вереск накрывает и лес), и
    /// младший не сажает там, где лежит старший. Лес — 0.
    rank: u8,
}

fn growth(t: &Tags) -> Option<Growth> {
    let (landuse, natural) = (get(t, "landuse"), get(t, "natural"));
    if natural == "wood" || landuse == "forest" {
        // Хвойный — зелёные сосны, лиственный — летние, смешанный — вперемешку.
        let slots: &[&str] = match get(t, "leaf_type") {
            "needleleaved" => &["pine"],
            "broadleaved" => &["tree_green"],
            _ => &["tree_green", "tree_green", "pine"],
        };
        return Some(Growth { slots, spacing: 3.4, scale: (1.0, 1.6), share: 1.0, litter: true, rank: 0 });
    }
    if landuse == "orchard" {
        return Some(Growth { slots: &["tree_green"], spacing: 4.5, scale: (0.7, 0.9), share: 1.0, litter: false, rank: 1 });
    }
    if natural == "scrub" {
        // Кустарник сверху — сплошной полог кустов и подроста (на снимке Лулворта
        // не отличить от леса); редкие кусты на бурой траве выглядели пустошью.
        return Some(Growth { slots: &["bush_summer", "bush_summer", "bush_summer", "tree_green", "tree_green"], spacing: 1.9, scale: (0.8, 1.4), share: 0.95, litter: false, rank: 2 });
    }
    if natural == "wetland" {
        return Some(Growth { slots: &["cattail", "cattail", "bush_summer"], spacing: 2.0, scale: (0.7, 1.1), share: 0.4, litter: false, rank: 3 });
    }
    if natural == "heath" {
        // Пустошь сверху — папоротник и вереск куртинами, кусты и редкие молодые деревья;
        // голая трава с камнями была пустой против снимка. Папоротник — только из паков.
        // Деревьев мало: в OSM пустошь — кустарнички, вереск, папоротник, а пастбища
        // Малхема с тем же тегом на снимке — открытая трава; деревья по всей пустоши
        // делали из неё редколесье.
        return Some(Growth {
            slots: &["fern", "fern", "fern", "fern", "fern", "bush_summer", "bush_summer", "bush_summer", "tree_green", "boulder"],
            spacing: 2.6,
            scale: (0.6, 1.0),
            share: 0.4,
            litter: false,
            rank: 4,
        });
    }
    // Осыпь — россыпь камней, скала — редкие глыбы; порода по тегу `rock`. Старше
    // пустоши и кустарника: скала внутри пастбища не зарастает кустами.
    if natural == "scree" {
        return Some(Growth { slots: &["boulder"], spacing: 2.4, scale: (0.3, 0.7), share: 0.5, litter: false, rank: 1 });
    }
    if matches!(natural, "bare_rock" | "stone") {
        return Some(Growth { slots: &["boulder"], spacing: 4.5, scale: (0.4, 0.9), share: 0.2, litter: false, rank: 1 });
    }
    None
}

/// Голый камень: скала, осыпь, камни.
fn growth_rock(t: &Tags) -> bool {
    matches!(get(t, "natural"), "bare_rock" | "scree" | "stone")
}

/// Рамка колец, обрезанная по карте; пустая — контур весь за краем.
fn clipped_box(rings: &[Vec<[f32; 2]>], w: f32, h: f32) -> Option<([f32; 2], [f32; 2])> {
    let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
    for p in rings.iter().flatten() {
        for c in 0..2 {
            lo[c] = lo[c].min(p[c]);
            hi[c] = hi[c].max(p[c]);
        }
    }
    let (lo, hi) = ([lo[0].max(0.0), lo[1].max(0.0)], [hi[0].min(w), hi[1].min(h)]);
    (hi[0] > lo[0] && hi[1] > lo[1]).then_some((lo, hi))
}

/// Внутри колец (чётно-нечётно).
fn inside(rings: &[Vec<[f32; 2]>], p: [f32; 2]) -> bool {
    let mut odd = false;
    for r in rings {
        for k in 0..r.len() {
            let (a, b) = (r[k], r[(k + 1) % r.len()]);
            if (a[1] > p[1]) != (b[1] > p[1]) && p[0] < a[0] + (p[1] - a[1]) * (b[0] - a[0]) / (b[1] - a[1]) {
                odd = !odd;
            }
        }
    }
    odd
}


/// Площадь, перрон, мощёный участок дороги: место, где ходят по всей ширине.
fn plaza(t: &Tags) -> bool {
    get(t, "railway") == "platform" || (get(t, "highway") == "pedestrian" && get(t, "area") == "yes") || get(t, "place") == "square" || t.contains_key("area:highway")
}

/// Парк или сад, чью траву решает только `leisure`: газон и луг у него не
/// отмечены. Такой контур в городе обнимает и дорожки, и мощёные площадки.
fn park_only(t: &Tags) -> bool {
    matches!(get(t, "leisure"), "park" | "garden") && get(t, "landuse").is_empty() && get(t, "natural").is_empty()
}

/// Площади, что в OSM «грунт» (`surface=dirt` и подобное), а на снимке выглядят
/// как мощёные площади той же карты, — мостовой самой большой из них: номер
/// контура и текстура. Правка StreetComplete от 24.08.2026, сделанная на глаз
/// (одна правка от Киева до Москвы), положила грунт на Васильевский спуск и
/// Соборную площадь, а на снимке 2025 года там брусчатка. Мерило — мощёное этого
/// же снимка, а не «серость»: светлая земля пустыни на снимке серее асфальта.
fn relay_plazas(areas: &[Area], m: &Map, hint: Option<&Hint>, sea: bool, warnings: &mut Vec<String>) -> HashMap<usize, &'static str> {
    let mut out = HashMap::new();
    let Some(hint) = hint else { return out };
    let paved = |t: &Tags| matches!(get(t, "surface"), "asphalt" | "paved" | "concrete" | "concrete:plates" | "paving_stones" | "sett" | "cobblestone" | "unhewn_cobblestone" | "granite" | "chipseal");
    let unpaved = |t: &Tags| matches!(get(t, "surface"), "dirt" | "ground" | "earth" | "unpaved" | "mud");
    let mw = m.rooms.w;
    let reference: Vec<(usize, Vec<(usize, f32)>)> = areas
        .iter()
        .enumerate()
        .filter(|(_, a)| plaza(a.tags) && paved(a.tags) && building(a.tags).is_none())
        .map(|(k, a)| (k, coverage(&m.rooms, &a.rings, 0.0)))
        .filter(|(_, c)| !c.is_empty())
        .collect();
    let Some(largest) = reference.iter().max_by_key(|r| r.1.len()) else { return out };
    let all: Vec<(usize, f32)> = reference.iter().flat_map(|r| r.1.iter().copied()).collect();
    let Some(norm) = hint.tone(&all, mw) else { return out };
    let tex = ground(areas[largest.0].tags, sea).map_or("street", |g| g.0);
    for (k, a) in areas.iter().enumerate().filter(|(_, a)| plaza(a.tags) && unpaved(a.tags) && building(a.tags).is_none()) {
        let Some(seen) = hint.tone(&coverage(&m.rooms, &a.rings, 0.0), mw) else { continue };
        if (seen.chroma - norm.chroma).abs() < 8.0 && (0.7..1.45).contains(&(seen.value / norm.value.max(1.0))) {
            out.insert(k, tex);
            let name = match get(a.tags, "name") {
                "" => "площадь",
                n => n,
            };
            warnings.push(format!("{name}: в OSM грунт (surface={}), а на снимке как мощёное рядом — положена мостовая", get(a.tags, "surface")));
        }
    }
    out
}

/// Цвет из тега OSM: `#rgb`, `#rrggbb` или ходовое в OSM имя CSS.
fn colour(v: &str) -> Option<[u8; 3]> {
    let v = v.trim().to_lowercase();
    if let Some(hex) = v.strip_prefix('#') {
        let d: Vec<u8> = hex.chars().filter_map(|c| c.to_digit(16).map(|d| d as u8)).collect();
        return match (hex.len(), d.len()) {
            (3, 3) => Some([d[0] * 17, d[1] * 17, d[2] * 17]),
            (6, 6) => Some([d[0] * 16 + d[1], d[2] * 16 + d[3], d[4] * 16 + d[5]]),
            _ => None,
        };
    }
    Some(match v.as_str() {
        "white" => [255, 255, 255],
        "ivory" => [255, 255, 240],
        "beige" => [245, 245, 220],
        "cream" => [255, 253, 208],
        "yellow" => [255, 255, 0],
        "gold" => [255, 215, 0],
        "orange" => [255, 165, 0],
        "tan" => [210, 180, 140],
        "brown" => [165, 42, 42],
        "sienna" => [160, 82, 45],
        "maroon" => [128, 0, 0],
        "darkred" => [139, 0, 0],
        "red" => [255, 0, 0],
        "firebrick" => [178, 34, 34],
        "indianred" => [205, 92, 92],
        "salmon" => [250, 128, 114],
        "darksalmon" => [233, 150, 122],
        "pink" => [255, 192, 203],
        "grey" | "gray" => [128, 128, 128],
        "darkgrey" | "darkgray" => [169, 169, 169],
        "lightgrey" | "lightgray" => [211, 211, 211],
        "silver" => [192, 192, 192],
        "black" => [0, 0, 0],
        "green" => [0, 128, 0],
        "darkgreen" => [0, 100, 0],
        "lightgreen" => [144, 238, 144],
        "olive" => [128, 128, 0],
        "blue" => [0, 0, 255],
        "lightblue" => [173, 216, 230],
        "navy" => [0, 0, 128],
        "teal" => [0, 128, 128],
        _ => return None,
    })
}

/// Крепостная стена контуром (`building=wall`, `historic=citywalls`): сверху её
/// видно целиком — боевой ход с зубцами, а не дом с дощатым полом и дверью.
fn rampart(t: &Tags) -> bool {
    get(t, "building") == "wall" || matches!(get(t, "historic"), "citywalls" | "city_wall")
}

/// Зубцы по краю боевого хода: низкие стенки 1,3 × 0,4 клетки с шагом 2,2 у самой
/// кромки — через них не пройти, но видно. Заливкой пола они расплывались в
/// пятна: в клетке четыре точки маски, а зубец — полторы. Край, за которым
/// другой дом (башня, пристройка), — без зубцов.
fn battlements(m: &mut Map, rings: &[Vec<[f32; 2]>], style: &WallStyle) {
    const STEP: f32 = 2.2;
    let res = mask::RES as f32;
    let inset = 0.12 + style.width / 2.0;
    let mut teeth: Vec<Wall> = Vec::new();
    for r in rings {
        for k in 0..r.len() {
            let (a, b) = (r[k], r[(k + 1) % r.len()]);
            let len = length(a, b);
            if len < 1.6 {
                continue;
            }
            let dir = [(b[0] - a[0]) / len, (b[1] - a[1]) / len];
            let mid = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
            let mut n = [-dir[1], dir[0]];
            if !inside(rings, [mid[0] + n[0] * 0.3, mid[1] + n[1] * 0.3]) {
                n = [-n[0], -n[1]];
            }
            let out = [mid[0] - n[0] * 0.6, mid[1] - n[1] * 0.6];
            if m.rooms.get((out[0] * res).floor() as isize, (out[1] * res).floor() as isize) >= 128 {
                continue;
            }
            let count = ((len - 0.4) / STEP).floor().max(1.0) as usize;
            let start = (len - (count as f32 - 1.0) * STEP) / 2.0;
            for i in 0..count {
                let s = start + i as f32 * STEP;
                let at = |t: f32| [a[0] + dir[0] * t + n[0] * inset, a[1] + dir[1] * t + n[1] * inset];
                let (p, q) = (at(s - 0.65), at(s + 0.65));
                if m.contains(p[0].floor() as i32, p[1].floor() as i32) && m.contains(q[0].floor() as i32, q[1].floor() as i32) {
                    teeth.push(Wall { points: vec![p, q], closed: false, bends: Vec::new(), low: true, style: Some(style.clone()) });
                }
            }
        }
    }
    m.walls.extend(teeth);
}

/// Вид дома: пол, слот стены, деревянные ли двери.
fn building_look(kind: &str, t: &Tags) -> (&'static str, &'static str, bool) {
    // Пол дома не должен совпадать с мостовой: у ГУМа и Мавзолея он был той же
    // серой плиткой, что и Красная площадь, и дом читался лишь по тонкой стене.
    let floor = match kind {
        "church" | "cathedral" | "chapel" | "mosque" | "temple" | "synagogue" | "shrine" | "monastery" | "mausoleum" => "temple_floor",
        "industrial" | "warehouse" | "garage" | "garages" | "hangar" | "tower" | "castle" | "fortress" | "bunker" | "train_station" | "transportation" => "brick_floor",
        _ => "wood_planks",
    };
    let wall = match get(t, "building:material") {
        "stone" | "sandstone" | "limestone" | "granite" => "wall_stone_blocks",
        "wood" | "timber_framing" => "wall_timber",
        "metal" | "steel" | "glass" => "wall_metal",
        _ => match kind {
            "church" | "cathedral" | "chapel" => "wall_stone_blocks",
            "hut" | "cabin" | "shed" => "wall_wood",
            _ => "wall_brick",
        },
    };
    (floor, wall, matches!(wall, "wall_wood" | "wall_timber") || floor == "wood_planks")
}

/// Дом ли это и какой: `building=*`, кроме навесов и недостроенного.
fn building(t: &Tags) -> Option<&str> {
    let b = get(t, "building");
    (!b.is_empty() && !matches!(b, "no" | "roof" | "construction" | "proposed" | "demolished" | "ruins")).then_some(b)
}

/// Чем выложена дорога: по тегу `surface`, без него — по виду дороги и месту
/// (в городе тротуар плиткой, за городом тропа — земля). Проезжая мощёная —
/// булыжник: асфальта в D&D нет. Утоптанный гравий — светлый, как на снимке;
/// травяная тропа — примятая бурая трава, а не земля.
fn paving(t: &Tags, hw: &str, rural: bool) -> &'static str {
    let car = !matches!(hw, "pedestrian" | "footway" | "path" | "steps" | "cycleway" | "bridleway" | "track");
    match get(t, "surface") {
        "asphalt" | "paved" | "concrete" | "concrete:plates" | "paving_stones" | "chipseal" => {
            if car {
                "street"
            } else {
                "stone_tiles"
            }
        }
        "sett" | "cobblestone" | "unhewn_cobblestone" => "street",
        "gravel" | "fine_gravel" | "compacted" => "path_gravel",
        "pebblestone" | "rock" => "gravel",
        "grass" | "grass_paver" => "rough_grass",
        "dirt" | "ground" | "earth" | "unpaved" | "woodchips" => "dirt",
        "mud" => "mud",
        "sand" => "sand",
        _ if car => "street",
        _ if rural || matches!(hw, "track" | "path" | "bridleway") => "dirt",
        _ => "stone_tiles",
    }
}

/// Не мощёное: край у такой дорожки мягкий и неровный.
fn natural_surface(tex: &str) -> bool {
    matches!(tex, "dirt" | "gravel" | "path_gravel" | "rough_grass" | "mud" | "sand")
}

/// Улица: ширина в метрах, текстура, порядок (выше — поверх).
fn road(t: &Tags, rural: bool) -> Option<(f32, &'static str, u8)> {
    if get(t, "area") == "yes" {
        return None;
    }
    let hw = get(t, "highway");
    let w = match hw {
        "motorway" | "trunk" => 12.0,
        "primary" => 10.0,
        "secondary" => 9.0,
        "tertiary" => 8.0,
        "motorway_link" | "trunk_link" | "primary_link" | "secondary_link" | "tertiary_link" => 7.0,
        "unclassified" | "residential" => 6.0,
        "living_street" | "pedestrian" => 5.0,
        "service" => 4.0,
        "track" => 3.0,
        "footway" | "steps" | "cycleway" | "bridleway" => 2.0,
        "path" => 1.5,
        _ => return None,
    };
    let order = if matches!(hw, "pedestrian" | "footway" | "path" | "steps" | "cycleway" | "bridleway" | "track") { 3 } else { 4 };
    Some((width_tag(t).unwrap_or(w), paving(t, hw, rural), order))
}

/// Рельсовый путь: колея в метрах и есть ли насыпь (у трамвая рельсы в мостовой).
fn rail(t: &Tags) -> Option<(f32, bool)> {
    let ballast = match get(t, "railway") {
        "rail" | "light_rail" | "narrow_gauge" | "subway" | "disused" | "preserved" | "funicular" => true,
        "tram" => false,
        _ => return None,
    };
    let gauge = get(t, "gauge").split(';').next().and_then(|g| g.trim().parse::<f32>().ok()).filter(|g| (500.0..2000.0).contains(g)).unwrap_or(1435.0);
    Some((gauge / 1000.0, ballast))
}

/// Ручей, канал, река: ширина в метрах и глубокая ли вода.
fn stream(t: &Tags) -> Option<(f32, bool)> {
    let (w, deep) = match get(t, "waterway") {
        "river" => (20.0, true),
        "canal" => (12.0, true),
        "stream" => (3.0, false),
        "ditch" | "drain" => (1.5, false),
        _ => return None,
    };
    Some((width_tag(t).unwrap_or(w), deep))
}

/// Ломаная, сдвинутая вбок на `d` клеток (плюс — влево по ходу).
fn offset(pts: &[[f32; 2]], d: f32) -> Vec<[f32; 2]> {
    let n = pts.len();
    (0..n)
        .map(|k| {
            let (a, b) = (pts[k.saturating_sub(1)], pts[(k + 1).min(n - 1)]);
            let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
            let len = (dx * dx + dy * dy).sqrt().max(1e-6);
            [pts[k][0] + dy / len * d, pts[k][1] - dx / len * d]
        })
        .collect()
}

/// Куски ломаной у карты: точки дальше `margin` клеток от края выбрасываются,
/// кроме соседей тех, что внутри, — линия уходит за край, а не обрывается на нём.
fn near_map(pts: &[[f32; 2]], w: f32, h: f32, margin: f32) -> Vec<Vec<[f32; 2]>> {
    let inside = |p: [f32; 2]| p[0] >= -margin && p[1] >= -margin && p[0] <= w + margin && p[1] <= h + margin;
    let keep: Vec<bool> = (0..pts.len()).map(|k| inside(pts[k]) || (k > 0 && inside(pts[k - 1])) || (k + 1 < pts.len() && inside(pts[k + 1]))).collect();
    let mut out: Vec<Vec<[f32; 2]>> = Vec::new();
    let mut cur = Vec::new();
    for (k, &p) in pts.iter().enumerate() {
        if keep[k] {
            if cur.last() != Some(&p) {
                cur.push(p);
            }
        } else if !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
    }
    out.push(cur);
    out.retain(|l| l.len() >= 2);
    out
}

/// Сила точек маски внутри колец (чётно-нечётно — двор внутри дома остаётся
/// двором) с краем на шаг маски, как у овальной кисти: контур ляжет ровно по
/// линии, и стена под углом выйдет прямой. Только то, что на карте. `inset` —
/// на сколько клеток край отступает внутрь (середина озера без мелководья).
///
/// Внутри или нет — построчно по пересечениям строки с рёбрами; расстояние до
/// края — только до рёбер из соседних корзин в клетку: у озера и вереска рёбер
/// тысячи, а каждую точку со всеми не сравнить.
fn coverage(mk: &Mask, rings: &[Vec<[f32; 2]>], inset: f32) -> Vec<(usize, f32)> {
    let res = mask::RES as f32;
    let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
    for p in rings.iter().flatten() {
        for c in 0..2 {
            lo[c] = lo[c].min(p[c]);
            hi[c] = hi[c].max(p[c]);
        }
    }
    let span = |lo: f32, hi: f32, n: usize| (((lo * res).floor() as isize - 1).clamp(0, n as isize) as usize, ((hi * res).ceil() as isize + 1).clamp(0, n as isize) as usize);
    let (i0, i1) = span(lo[0], hi[0], mk.w);
    let (j0, j1) = span(lo[1], hi[1], mk.h);
    let edges: Vec<([f32; 2], [f32; 2])> = rings.iter().flat_map(|r| (0..r.len()).map(move |k| (r[k], r[(k + 1) % r.len()]))).collect();
    let mut buckets: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
    for (e, &(a, b)) in edges.iter().enumerate() {
        let n = (length(a, b) / 0.5).ceil().max(1.0) as usize;
        for s in 0..=n {
            let f = s as f32 / n as f32;
            let key = ((a[0] + (b[0] - a[0]) * f).floor() as i32, (a[1] + (b[1] - a[1]) * f).floor() as i32);
            let v = buckets.entry(key).or_default();
            if v.last() != Some(&e) {
                v.push(e);
            }
        }
    }
    let reach = (inset + 1.0).ceil() as i32;
    let aa = 0.5 / res;
    let mut out = Vec::new();
    let mut xs: Vec<f32> = Vec::new();
    for j in j0..j1 {
        let y = (j as f32 + 0.5) / res;
        xs.clear();
        xs.extend(edges.iter().filter(|(a, b)| (a[1] > y) != (b[1] > y)).map(|(a, b)| a[0] + (y - a[1]) * (b[0] - a[0]) / (b[1] - a[1])));
        xs.sort_by(f32::total_cmp);
        let mut left = 0;
        for i in i0..i1 {
            let x = (i as f32 + 0.5) / res;
            while left < xs.len() && xs[left] < x {
                left += 1;
            }
            let (cx, cy) = (x.floor() as i32, y.floor() as i32);
            let mut d = f32::INFINITY;
            for dy in -reach..=reach {
                for dx in -reach..=reach {
                    for &e in buckets.get(&(cx + dx, cy + dy)).map_or(&[][..], |v| v.as_slice()) {
                        d = d.min(segment_distance([x, y], edges[e].0, edges[e].1));
                    }
                }
            }
            let s = if left % 2 == 1 { d - inset } else { -d - inset };
            let v = (0.5 + s / (2.0 * aa)).clamp(0.0, 1.0);
            if v > 0.0 {
                out.push((j * mk.w + i, v));
            }
        }
    }
    out
}

/// Плавный шум места от −1 до 1: ячейка в `scale` клеток, `seed` — какое из полей.
/// Зависит только от места, а не от порядка обхода.
fn noise(p: [f32; 2], scale: f32, seed: u32) -> f32 {
    let hash = |x: i32, y: i32| {
        let mut h = (x as u32).wrapping_mul(374_761_393) ^ (y as u32).wrapping_mul(668_265_263) ^ seed.wrapping_mul(2_246_822_519);
        h = (h ^ (h >> 13)).wrapping_mul(1_274_126_177);
        ((h ^ (h >> 16)) & 0xffff) as f32 / 65535.0 * 2.0 - 1.0
    };
    let (gx, gy) = (p[0] / scale, p[1] / scale);
    let (x0, y0) = (gx.floor() as i32, gy.floor() as i32);
    let (fx, fy) = (gx - x0 as f32, gy - y0 as f32);
    let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let top = hash(x0, y0) * (1.0 - sx) + hash(x0 + 1, y0) * sx;
    let bottom = hash(x0, y0 + 1) * (1.0 - sx) + hash(x0 + 1, y0 + 1) * sx;
    top * (1.0 - sy) + bottom * sy
}

/// Сдвиг природного контура в точке: плавное поле шума, около полуклетки. Поле
/// зависит только от места — общая граница леса и вереска сдвигается одинаково.
fn warp(p: [f32; 2]) -> [f32; 2] {
    const SCALE: f32 = 5.0;
    const AMP: f32 = 0.7;
    [noise(p, SCALE, 1) * AMP, noise(p, SCALE, 2) * AMP]
}

/// Природный контур: рёбра дробятся на куски до двух клеток, точки сдвигаются
/// полем `warp`, углы срезаются дважды не дальше полутора клеток — берег и
/// опушка без прямых по линейке. Дробление от концов ребра одинаково в обе
/// стороны, поэтому общая граница соседей остаётся общей. Точки, где сходятся
/// несколько контуров (`pinned`), не срезаются: иначе у стыка двух кусков реки
/// каждый скругляет свой угол и на берегу остаётся выемка суши.
fn natural_outline(r: &[[f32; 2]], pinned: &[bool]) -> Vec<[f32; 2]> {
    let mut pts = Vec::new();
    let mut pins = Vec::new();
    for k in 0..r.len() {
        let (a, b) = (r[k], r[(k + 1) % r.len()]);
        let n = (length(a, b) / 2.0).ceil().max(1.0) as usize;
        for s in 0..n {
            let f = s as f32 / n as f32;
            let p = [a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f];
            let d = warp(p);
            pts.push([p[0] + d[0], p[1] + d[1]]);
            pins.push(s == 0 && pinned.get(k).copied().unwrap_or(false));
        }
    }
    for _ in 0..2 {
        let n = pts.len();
        let (mut next, mut next_pins) = (Vec::with_capacity(n * 2), Vec::with_capacity(n * 2));
        for k in 0..n {
            let (u, v, w) = (pts[(k + n - 1) % n], pts[k], pts[(k + 1) % n]);
            if pins[k] {
                next.push(v);
                next_pins.push(true);
                continue;
            }
            let toward = |q: [f32; 2]| {
                let t = (1.5 / length(v, q).max(1e-6)).min(0.25);
                [v[0] + (q[0] - v[0]) * t, v[1] + (q[1] - v[1]) * t]
            };
            next.extend([toward(u), toward(w)]);
            next_pins.extend([false, false]);
        }
        (pts, pins) = (next, next_pins);
    }
    pts
}

/// Линия с точками не реже `step` клеток. Дорога в Картографе тает к концам на
/// ширину края, и прямая из двух точек OSM таяла целиком — её не было видно.
fn dense(line: &[[f32; 2]], step: f32) -> Vec<[f32; 2]> {
    let mut out = Vec::with_capacity(line.len());
    for s in line.windows(2) {
        let n = (length(s[0], s[1]) / step).ceil().max(1.0) as usize;
        out.extend((0..n).map(|q| {
            let f = q as f32 / n as f32;
            [s[0][0] + (s[1][0] - s[0][0]) * f, s[0][1] + (s[1][1] - s[0][1]) * f]
        }));
    }
    out.extend(line.last().copied());
    out
}

/// Середина взвешенных точек маски и их площадь в клетках.
fn centre(mk: &Mask, weights: &[(usize, f32)]) -> ([f32; 2], f32) {
    let (mut sx, mut sy, mut sw) = (0.0, 0.0, 0.0);
    for &(k, v) in weights {
        let p = mask::sample_pos((k % mk.w) as isize, (k / mk.w) as isize);
        sx += p[0] * v;
        sy += p[1] * v;
        sw += v;
    }
    let cells = sw / (mask::RES * mask::RES) as f32;
    ([sx / sw.max(1e-6), sy / sw.max(1e-6)], cells)
}

fn length(a: [f32; 2], b: [f32; 2]) -> f32 {
    ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt()
}

/// Поворот подписи вдоль отрезка, чтобы читалась слева направо.
fn upright(a: [f32; 2], b: [f32; 2]) -> f32 {
    let deg = (b[1] - a[1]).atan2(b[0] - a[0]).to_degrees();
    if deg > 90.0 {
        deg - 180.0
    } else if deg <= -90.0 {
        deg + 180.0
    } else {
        deg
    }
}

/// Самый длинный отрезок ломаной.
fn longest(pts: &[[f32; 2]]) -> Option<([f32; 2], [f32; 2])> {
    pts.windows(2).map(|w| (w[0], w[1])).max_by(|x, y| length(x.0, x.1).total_cmp(&length(y.0, y.1)))
}

/// Пол по слоту — один вариант на карту: с паками FA каждая улица иначе брала бы
/// свою брусчатку.
#[derive(Default)]
struct Floors(BTreeMap<&'static str, usize>);

impl Floors {
    fn get(&mut self, m: &mut Map, slot: &'static str) -> usize {
        *self.0.entry(slot).or_insert_with(|| {
            let k = mat(m, slot);
            // Трава, земля и вода лежат полями на сотни клеток: без перемешивания
            // плитки (как у встроенных) по лугу идёт узор повтора.
            if slot.starts_with("rock_") || matches!(slot, "grass_green" | "grass_dry" | "meadow" | "rough_grass" | "dirt" | "gravel" | "shingle" | "path_gravel" | "sand" | "mud" | "forest_floor" | "swamp" | "rocky_ground" | "water_shallow" | "water_deep") {
                m.materials[k].vary = true;
            }
            k
        })
    }

    /// Пол слота в цвете `rgb` (см. [`library::tint_key`]): тот же вариант
    /// картинки, что у [`Floors::get`], перекрашенный.
    fn tinted(&mut self, m: &mut Map, slot: &'static str, rgb: [u8; 3]) -> usize {
        let k = self.get(m, slot);
        let base = m.materials[k].clone();
        let paint = match &base.paint {
            Paint::Texture(key) => Paint::Texture(library::tint_key(rgb, key)),
            Paint::Color(c) => Paint::Color([0, 1, 2].map(|i| ((c[i] as u32 + rgb[i] as u32) / 2) as u8)),
        };
        m.material_index(Material { paint, ..base }).unwrap_or(k)
    }
}

/// Стена слота в цвете `rgb`, если он есть.
fn wall_tinted(slot: &str, rgb: Option<[u8; 3]>) -> WallStyle {
    let mut style = wall(slot);
    if let (Some(c), Paint::Texture(key)) = (rgb, &style.paint) {
        style.paint = Paint::Texture(library::tint_key(c, key));
    }
    style
}

/// Уличный предмет по точке OSM: фонарь, скамья, фонтан, памятник.
fn furniture(t: &Tags) -> Option<&'static str> {
    if get(t, "highway") == "street_lamp" {
        return Some("street_lamp");
    }
    match get(t, "amenity") {
        "bench" => return Some("bench"),
        "fountain" => return Some("fountain"),
        _ => {}
    }
    let statue = matches!(get(t, "artwork_type"), "statue" | "sculpture") || matches!(get(t, "memorial"), "statue" | "war_memorial" | "obelisk");
    (statue && (get(t, "tourism") == "artwork" || !get(t, "historic").is_empty())).then_some("statue")
}

/// Ограда по линии OSM: слот стены, низкая ли (видно поверх), толщина в клетках.
/// За городом стена — сухая кладка из бута по пояс, в городе — высокая из блоков.
fn barrier(t: &Tags, rural: bool) -> Option<(&'static str, bool, f32)> {
    match get(t, "barrier") {
        "wall" | "retaining_wall" if rural => Some(("wall_fieldstone", true, 0.35)),
        "wall" | "city_wall" | "retaining_wall" => Some(("wall_stone_blocks", false, 0.3)),
        "fence" => Some(("wall_wood", true, 0.15)),
        "railing" | "guard_rail" | "handrail" => Some(("wall_metal", true, 0.12)),
        "hedge" => Some(("wall_hedge", false, 0.45)),
        // Обрыв — не стена, а рельеф (`cliff_relief`): полоса скалы, тень и стена в Foundry.
        _ => None,
    }
}

/// Ярусов у обрыва без тега `height` и предел: обрыв Малхема в 80 м — полсотни
/// ярусов, а сверху стенка всё равно отвесная, и полоса в десятки клеток съела бы карту.
const CLIFF_LEVELS: f32 = 3.0;
const CLIFF_MAX: f32 = 6.0;
/// Плато за кромкой обрыва, клетки; дальше земля спадает на `CLIFF_FALL` яруса
/// на клетку — пологим склоном, а не вторым обрывом. Круче (0,35) склон рисовался
/// кольцами гребней, на Лулворте — поперёк пляжа.
const CLIFF_SHELF: f32 = 3.0;
const CLIFF_FALL: f32 = 0.15;
/// За концом линии обрыв сходит на нет на этом пути, клетки.
const CLIFF_END: f32 = 2.0;
/// Низина за концом линии поднимается так полого (ярусов на клетку) — без стенки и гребня.
const CLIFF_END_FALL: f32 = 0.25;

/// Высота обрыва в ярусах (ярус — клетка, 1,524 м) по тегу `height`.
fn cliff_levels(t: &Tags) -> f32 {
    get(t, "height").trim().trim_end_matches('m').trim().parse::<f32>().ok().filter(|v| *v > 0.0).map_or(CLIFF_LEVELS, |m| (m / CELL_M).round().clamp(1.0, CLIFF_MAX))
}

/// Рельеф по обрывам OSM. Верх обрыва в OSM — слева по ходу линии при севере
/// сверху; на карте, где y вниз, это нормаль `(dy, −dx)` к ходу `(dx, dy)`
/// (линия на восток — верх на севере). Опускается нижняя сторона, а не
/// поднимается верхняя: рельеф темнит ярусы ниже самого верхнего, и узкое
/// поднятое плато затемняло всю карту вокруг. Низ — полоса `CLIFF_SHELF`, дальше
/// плавный подъём обратно; за концами линии — на нет за `CLIFF_END`. Кромки,
/// полосы, тени и стены Foundry дальше строит рельеф сам.
fn cliff_relief(lines: &[(Vec<[f32; 2]>, f32)], w: u32, h: u32) -> Option<crate::relief::Relief> {
    if lines.is_empty() {
        return None;
    }
    let mut r = crate::relief::Relief::for_cells(w, h);
    // Настоящая местность: низина не темнеет по высоте, пологий подъём — без гребней.
    r.natural = true;
    let res = mask::RES as f32;
    for (line, levels) in lines {
        // За концом линии стенка сходит на нет за `CLIFF_END`, а низина — полого,
        // `CLIFF_END_FALL` яруса на клетку: при одном коротком спаде торец низины
        // вставал стенкой поперёк пляжа Лулворта и дальше в море.
        let end_len = (levels / CLIFF_END_FALL).max(CLIFF_END);
        let reach = CLIFF_SHELF + levels / CLIFF_FALL + end_len;
        let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
        for p in line {
            for c in 0..2 {
                lo[c] = lo[c].min(p[c] - reach);
                hi[c] = hi[c].max(p[c] + reach);
            }
        }
        let i0 = ((lo[0] * res).floor().max(0.0) as usize).min(r.w);
        let i1 = ((hi[0] * res).ceil().max(0.0) as usize).min(r.w);
        let j0 = ((lo[1] * res).floor().max(0.0) as usize).min(r.h);
        let j1 = ((hi[1] * res).ceil().max(0.0) as usize).min(r.h);
        let n = line.len() - 1;
        for j in j0..j1 {
            for i in i0..i1 {
                let p = [(i as f32 + 0.5) / res, (j as f32 + 0.5) / res];
                // Ближайший отрезок; у общей вершины — тот, от чьей прямой точка дальше.
                let (mut best, mut side, mut beyond) = (f32::INFINITY, 0.0f32, 0.0f32);
                for (k, s) in line.windows(2).enumerate() {
                    let (a, b) = (s[0], s[1]);
                    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
                    let len = dx.hypot(dy).max(1e-6);
                    let t = ((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / (len * len);
                    let q = [a[0] + dx * t.clamp(0.0, 1.0), a[1] + dy * t.clamp(0.0, 1.0)];
                    let d = (p[0] - q[0]).hypot(p[1] - q[1]);
                    let s = ((p[0] - a[0]) * dy - (p[1] - a[1]) * dx) / len;
                    if d < best - 1e-4 || (d < best + 1e-4 && s.abs() > side.abs()) {
                        best = best.min(d);
                        side = s;
                        beyond = if k == 0 && t < 0.0 { -t * len } else if k + 1 == n && t > 1.0 { (t - 1.0) * len } else { 0.0 };
                    }
                }
                let fall = if best <= CLIFF_SHELF { 1.0 } else { (1.0 - (best - CLIFF_SHELF) * CLIFF_FALL / levels).max(0.0) };
                let soft = (1.0 - beyond / end_len).max(0.0);
                let hard = (1.0 - beyond / CLIFF_END).max(0.0);
                let v = if side > 0.0 {
                    // Верх за концом линии опускается вслед за низиной: стенки там нет.
                    if beyond <= 0.0 {
                        continue;
                    }
                    -levels * fall * soft * (1.0 - hard)
                } else {
                    -levels * fall * soft
                };
                let k = j * r.w + i;
                r.data[k] = r.data[k].min(v);
            }
        }
    }
    Some(r)
}

/// Проход в ограде и его ширина в клетках: ворота, калитка, перелаз.
fn gate(t: &Tags) -> Option<f32> {
    match get(t, "barrier") {
        "gate" | "lift_gate" | "swing_gate" | "sliding_gate" | "hampshire_gate" | "cattle_grid" => Some(2.0),
        "stile" | "kissing_gate" | "entrance" | "turnstile" | "bump_gate" => Some(1.0),
        _ => None,
    }
}

/// Ограда кусками: у калиток и ворот — разрыв в их ширину.
fn split_at_gates(pts: &[[f32; 2]], gaps: &[Option<f32>]) -> Vec<Vec<[f32; 2]>> {
    let toward = |p: [f32; 2], q: [f32; 2], d: f32| {
        let l = length(p, q);
        (l > d).then(|| [p[0] + (q[0] - p[0]) * d / l, p[1] + (q[1] - p[1]) * d / l])
    };
    let mut out = Vec::new();
    let mut cur: Vec<[f32; 2]> = Vec::new();
    for (k, &p) in pts.iter().enumerate() {
        let Some(gap) = gaps[k] else {
            cur.push(p);
            continue;
        };
        if k > 0
            && let Some(q) = toward(p, pts[k - 1], gap / 2.0)
        {
            cur.push(q);
        }
        if cur.len() >= 2 {
            out.push(std::mem::take(&mut cur));
        }
        cur.clear();
        if k + 1 < pts.len()
            && let Some(q) = toward(p, pts[k + 1], gap / 2.0)
        {
            cur.push(q);
        }
    }
    if cur.len() >= 2 {
        out.push(cur);
    }
    out
}

/// Где сажать нельзя: дома, дороги, вода, ограды. Сетка маски (`RES` точек на клетку).
#[derive(Clone)]
struct Blocked {
    w: usize,
    h: usize,
    cells: Vec<bool>,
}

impl Blocked {
    fn new(m: &Map) -> Self {
        let cells = m.rooms.data.iter().map(|&v| v >= 128).collect();
        Self { w: m.rooms.w, h: m.rooms.h, cells }
    }

    fn disk(&mut self, c: [f32; 2], r: f32) {
        let res = mask::RES as f32;
        let (i0, i1) = (((c[0] - r) * res).floor().max(0.0) as usize, (((c[0] + r) * res).ceil().max(0.0) as usize).min(self.w));
        let (j0, j1) = (((c[1] - r) * res).floor().max(0.0) as usize, (((c[1] + r) * res).ceil().max(0.0) as usize).min(self.h));
        for j in j0..j1 {
            for i in i0..i1 {
                let p = mask::sample_pos(i as isize, j as isize);
                if (p[0] - c[0]).powi(2) + (p[1] - c[1]).powi(2) <= r * r {
                    self.cells[j * self.w + i] = true;
                }
            }
        }
    }

    /// Полоса вдоль ломаной.
    fn line(&mut self, pts: &[[f32; 2]], r: f32) {
        for s in pts.windows(2) {
            let n = (length(s[0], s[1]) / (r * 0.5).max(0.25)).ceil().max(1.0) as usize;
            for k in 0..=n {
                let f = k as f32 / n as f32;
                self.disk([s[0][0] + (s[1][0] - s[0][0]) * f, s[0][1] + (s[1][1] - s[0][1]) * f], r);
            }
        }
    }

    fn at(&self, p: [f32; 2]) -> bool {
        let res = mask::RES as f32;
        let (i, j) = ((p[0] * res).floor(), (p[1] * res).floor());
        i < 0.0 || j < 0.0 || i as usize >= self.w || j as usize >= self.h || self.cells[j as usize * self.w + i as usize]
    }
}

/// Сколько всего деревьев и кустов сажать на карту: больше — Foundry тяжело
/// (у каждой кроны свой тайл).
const MAX_PLANTS: usize = 3000;

/// Посаженное, разложенное по корзинам в клетку: не ближе шага к соседу.
#[derive(Default)]
struct Spacing(HashMap<(i32, i32), Vec<[f32; 2]>>);

impl Spacing {
    fn free(&self, q: [f32; 2], d: f32) -> bool {
        let (bx, by, r) = (q[0].floor() as i32, q[1].floor() as i32, d.ceil() as i32);
        (-r..=r).all(|dx| (-r..=r).all(|dy| self.0.get(&(bx + dx, by + dy)).is_none_or(|v| v.iter().all(|p| length(*p, q) >= d))))
    }

    fn add(&mut self, q: [f32; 2]) {
        self.0.entry((q[0].floor() as i32, q[1].floor() as i32)).or_default().push(q);
    }
}

/// Дом на карте: что про него нужно после заливки всех домов.
struct House {
    rings: Vec<Vec<[f32; 2]>>,
    /// Входы из OSM, в клетках.
    entrances: Vec<[f32; 2]>,
    style: WallStyle,
    wood: bool,
    /// Большой дом с широкими дверьми: вокзал, храм, магазин.
    grand: bool,
    /// Есть ли двери: у крепостной стены — нет.
    doors: bool,
}

/// Линия, которая ляжет дорогой: этаж, порядок и сами дороги снизу вверх.
type Lane = (f32, u8, Vec<Path>);

#[cfg(test)]
pub fn import(xml: &str, opt: Options) -> Result<(Map, Report), String> {
    import_cover(xml, &[], None, opt)
}

/// Карта из выгрузки и докачанных к ней контуров (`incomplete`); рамка — у выгрузки.
/// С подсказкой со снимка деревья — где на нём кроны, орляк и сухая трава — где
/// бурое, трава на голом камне — где он зарос.
pub fn import_cover(xml: &str, extra: &[String], cover: Option<&Cover>, opt: Options) -> Result<(Map, Report), String> {
    let mut osm = parse(xml)?;
    for e in extra {
        let _ = parse_into(&mut osm, e);
    }
    let [south, west, north, east] = osm.bounds;
    let mid = ((south + north) / 2.0).to_radians();
    let (mx, my) = (111_320.0 * mid.cos(), 110_574.0);
    let (wm, hm) = (((east - west) * mx) as f32, ((north - south) * my) as f32);
    let mut warnings = Vec::new();
    let mut cell = opt.cell_m.max(0.1);
    if wm.max(hm) / cell > opt.max_cells as f32 {
        cell = wm.max(hm) / opt.max_cells as f32;
        warnings.push(format!("участок {:.0}×{:.0} м больше {} клеток по 5 футов: клетка {:.1} м", wm, hm, opt.max_cells, cell));
    }
    // Природное, что так и не замкнулось (сети не было), — пропадает: сказать.
    let lost: Vec<String> = osm
        .relations
        .values()
        .filter(|r| get(&r.tags, "type") == "multipolygon" && !unseen(&r.tags) && (ground(&r.tags, false).is_some() || growth(&r.tags).is_some()))
        .filter(|r| r.ways.iter().any(|w| !osm.ways.contains_key(w)))
        .map(|r| match get(&r.tags, "name") {
            "" => ["natural", "landuse", "leisure"].iter().map(|k| get(&r.tags, k)).find(|v| !v.is_empty()).unwrap_or("контур").to_owned(),
            n => n.to_owned(),
        })
        .collect();
    if !lost.is_empty() {
        warnings.push(format!("неполные в выгрузке и пропущены: {}", lost.join(", ")));
    }
    // Десятая клетки сверху — округление границ в выгрузке (7 знаков: рамка
    // 40×30 клеток выходила 40,04×30,04 и на карте — 41×31 с полоской).
    let (w, h) = (((wm / cell - 0.1).ceil() as u32).max(8), ((hm / cell - 0.1).ceil() as u32).max(8));
    let proj = Proj { north, west, kx: mx / cell as f64, ky: my / cell as f64 };
    let hint = cover.map(|c| Hint { cover: c, span: [wm / cell, hm / cell] });
    let m_to_cells = |metres: f32| metres / cell;
    let mut m = Map::new(w, h);
    let sea_tags: Tags = [("natural", "water"), ("water", "sea")].into_iter().map(|(k, v)| (k.to_owned(), v.to_owned())).collect();
    let mut areas = areas(&osm, &proj);
    let sea = osm.ways.values().any(|w| get(&w.tags, "natural") == "coastline");
    let ground = |t: &Tags| ground(t, sea);
    // Природные контуры — с изгибом; дома, мощёное и пути остаются по линейке.
    // Узлы, общие для нескольких природных контуров, не срезаются.
    let natural: Vec<bool> = areas.iter().map(|a| building(a.tags).is_none() && !plaza(a.tags) && (growth(a.tags).is_some() || matches!(ground(a.tags), Some((t, _)) if !matches!(t, "stone_tiles" | "gravel" | "dirt" | "wood_planks")))).collect();
    let mut uses: HashMap<i64, u32> = HashMap::new();
    for a in areas.iter().zip(&natural).filter(|(_, n)| **n).map(|(a, _)| a) {
        for ring in &a.nodes {
            let mut seen = BTreeSet::new();
            for id in &ring[..ring.len() - 1] {
                if seen.insert(*id) {
                    *uses.entry(*id).or_default() += 1;
                }
            }
        }
    }
    for (a, _) in areas.iter_mut().zip(&natural).filter(|(_, n)| **n) {
        a.rings = a
            .rings
            .iter()
            .enumerate()
            .map(|(k, r)| {
                let pins: Vec<bool> = match a.nodes.get(k) {
                    Some(ids) if ids.len() == r.len() + 1 => ids[..r.len()].iter().map(|id| uses.get(id).copied().unwrap_or(0) > 1).collect(),
                    _ => vec![false; r.len()],
                };
                natural_outline(r, &pins)
            })
            .collect();
    }
    // Море по линии берега — обычной водой, с тем же изгибом берега.
    let sea_rings = sea_rings(&osm, &proj, w as f32, h as f32);
    if !sea_rings.is_empty() {
        let rings = sea_rings.iter().map(|r| natural_outline(r, &vec![false; r.len()])).collect();
        areas.push(Area { rings, nodes: Vec::new(), tags: &sea_tags });
    }
    // Город или природа — по доле застройки: от этого зависят пол под всем,
    // дорожки без тега покрытия и стены-ограды.
    // Застройка — только та, что на карте: в выгрузке дома и за её краем.
    let cells_of = |a: &Area| coverage(&m.rooms, &a.rings, 0.0).iter().map(|c| c.1).sum::<f32>() / (mask::RES * mask::RES) as f32;
    let built: f32 = areas.iter().filter(|a| building(a.tags).is_some()).map(cells_of).sum();
    // Боевая карта с хижиной или хутором (дом-три на рамку в 60 м, без городских
    // улиц и рельсов) — природа, хоть дома и занимают больше 6 % рамки.
    let houses = areas.iter().filter(|a| building(a.tags).is_some() && cells_of(a) > 0.0).count();
    let streets = osm.ways.values().any(|w| {
        matches!(get(&w.tags, "highway"), "residential" | "living_street" | "pedestrian" | "primary" | "secondary" | "tertiary" | "trunk") || !get(&w.tags, "railway").is_empty()
    });
    let rural = built < 0.06 * (w * h) as f32 || (houses <= 3 && !streets);
    m.ambience = if rural { Season::Summer.ambience() } else { "town-1" }.into();
    m.wall_style = wall(if rural { "wall_fieldstone" } else { "wall_brick" });

    // Под всем — мостовая или трава пятнами, поверх — земля, трава, вода и платформы.
    let mut floors = Floors::default();
    let glacier = areas.iter().any(|a| get(a.tags, "natural") == "glacier");
    let base_slot = if rural { hint.as_ref().map_or("meadow", |h| h.cover.base_floor()) } else { "flagstones" };
    // Снег и лёд по снимку: камень — серый сланец (мел и мергель выходили бежевыми),
    // тропа — примятый снег.
    let cold = matches!(base_slot, "ice_glacier" | "snow");
    // Снег и лёд по снимку — зимний звук места, а не летний лес.
    if rural && cold {
        m.ambience = Season::Winter.ambience().into();
    }
    // Верховое болото: бочаги — торфяная вода без кувшинок, мочажины — топь.
    let bog = base_slot == "bog_moss";
    // Сухой юг по снимку: лиственное — каменный дуб, сад — оливы, куст — ракитник,
    // эспарто, розмарин; папоротнику там не расти.
    let arid = matches!(base_slot, "earth_pale" | "straw_grass");
    let base = floors.get(&mut m, base_slot);
    let all = mask::cells(m.rooms.w, m.rooms.h, (0, 0), (w as i32 - 1, h as i32 - 1));
    m.paint(Layer::Floor, base, &all);
    let mut grounds: Vec<(u8, usize)> = areas.iter().enumerate().filter_map(|(k, a)| ground(a.tags).map(|(_, o)| (o, k))).collect();
    grounds.sort();
    let waters: Vec<usize> = grounds.iter().map(|g| g.1).filter(|&k| matches!(ground(areas[k].tags), Some(("water_deep", _)))).collect();
    let mut water_done = false;
    let mut rock_mats: Vec<usize> = Vec::new();
    let relaid = relay_plazas(&areas, &m, hint.as_ref(), sea, &mut warnings);
    for (_, k) in grounds {
        let (tex, _) = ground(areas[k].tags).unwrap_or(("flagstones", 0));
        let tex = relaid.get(&k).copied().unwrap_or(tex);
        // Ледник, что на снимке белый (фирн Мёнхсйоха), — снег: иначе по краю
        // ледника OSM оставалась ступенчатая полоса серого льда.
        let tex = if cold && tex.starts_with("rock_") {
            "rock_alpine"
        } else if base_slot == "snow" && tex == "ice_glacier" {
            "snow"
        } else {
            tex
        };
        if tex == "water_deep" {
            if !water_done {
                water(&mut m, &mut floors, &waters.iter().map(|&k| &areas[k]).collect::<Vec<_>>(), bog);
                water_done = true;
            }
            continue;
        }
        let weights = coverage(&m.rooms, &areas[k].rings, 0.0);
        // Парк и сад — травой только там, где она есть на снимке: в контуре парка
        // лежат и мощёные площадки. В Кремле газоном выходила площадь у белой
        // дорожки, а газоны OSM внутри того же парка со снимком совпадали.
        let weights = match &hint {
            Some(h) if park_only(areas[k].tags) => {
                let (mw, res) = (m.rooms.w, mask::RES as f32);
                weights
                    .into_iter()
                    .filter_map(|(i, v)| {
                        let p = [((i % mw) as f32 + 0.5) / res, ((i / mw) as f32 + 0.5) / res];
                        let t = ((h.green(p, 0.8) - 0.3) / 0.4).clamp(0.0, 1.0);
                        (t > 0.0).then_some((i, v * t * t * (3.0 - 2.0 * t)))
                    })
                    .collect()
            }
            _ => weights,
        };
        if !weights.is_empty() {
            // Бассейн и фонтан — на плитке: вода FA полупрозрачна, по траве
            // бассейн выходил тёмной кляксой.
            if tex == "water_shallow" {
                let tiles = floors.get(&mut m, "stone_tiles");
                m.paint(Layer::Floor, tiles, &weights);
            }
            let t = floors.get(&mut m, tex);
            m.paint(Layer::Floor, t, &weights);
            if tex.starts_with("rock_") && !rock_mats.contains(&t) {
                rock_mats.push(t);
            }
        }
    }
    // Отмели рек и озёр — по снимку: в OSM галечник у излучины чаще внутри
    // контура реки, и на карте там выходило мелководье по траве, серое, а на
    // снимке — светлая галька. У моря светлое у берега — пена прибоя, не галька.
    if let Some(hint) = &hint {
        let inland: Vec<&Area> = waters.iter().map(|&k| &areas[k]).filter(|a| get(a.tags, "water") != "sea").collect();
        let res = mask::RES as f32;
        let mw = m.rooms.w;
        let bars: Vec<(usize, f32)> = inland
            .iter()
            .flat_map(|a| coverage(&m.rooms, &a.rings, 0.0))
            .filter_map(|(k, v)| {
                let p = [((k % mw) as f32 + 0.5) / res, ((k / mw) as f32 + 0.5) / res];
                let rock = hint.share(p, 0.8, Class::Rock);
                let t = ((rock - 0.4) / 0.3).clamp(0.0, 1.0);
                (t > 0.0).then_some((k, v * t * t * (3.0 - 2.0 * t)))
            })
            .collect();
        if !bars.is_empty() {
            let t = floors.get(&mut m, "shingle");
            m.paint(Layer::Floor, t, &bars);
        }
    }
    if rural {
        // Луг пятнами некошеного и широкими разводами: на снимке поле не бывает
        // ровным ковром одного тона. Только по лугу — пляж, вода и пустошь целы.
        // И обратно: на пустоши — проплешины зелёной травы. Края — по полю шума в
        // две октавы: круглые пятна кистью выглядели наляпанными.
        let (meadow, rough) = (floors.get(&mut m, "meadow"), floors.get(&mut m, "rough_grass"));
        // Клетка годится, если она и соседи — того же пола: пятно не лезет на пляж и в воду.
        let only = |m: &Map, mat: usize| -> Vec<bool> {
            (0..(w * h) as i32)
                .map(|k| {
                    let (x, y) = (k % w as i32, k / w as i32);
                    [(0, 0), (1, 0), (-1, 0), (0, 1), (0, -1)].iter().all(|&(dx, dy)| matches!(m.floor_at(x + dx, y + dy) as usize, f if f == mat + 1 || f == 0))
                })
                .collect()
        };
        // На основе (луг, а вне зелёного биома — земля, сухая трава, снег).
        let on_ice = if glacier && base_slot != "snow" {
            let ice = floors.get(&mut m, "ice_glacier");
            only(&m, ice)
        } else {
            Vec::new()
        };
        let (mut to_earth, mut to_snow, mut to_rock, mut to_ice) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        // Голый камень — с травой в трещинах и на уступах: на снимке мел и сланец
        // местами заросли, сплошная серая заливка выглядела асфальтом.
        let on_rock: Vec<bool> = rock_mats.iter().map(|&r| only(&m, r)).fold(vec![false; (w * h) as usize], |acc, v| acc.iter().zip(v).map(|(a, b)| *a || b).collect());
        // Со снимком — по точкам маски, где лежит камень, вплоть до края контура:
        // проверка соседей по клеткам оставляла по краю голую полосу, на карте —
        // белый пунктир по заросшему склону.
        let rock_here: Vec<bool> = match &hint {
            Some(_) => (0..m.rooms.w * m.rooms.h).map(|k| m.terrain.iter().any(|c| rock_mats.contains(&(c.material as usize)) && c.mask.data[k] > 0)).collect(),
            None => Vec::new(),
        };
        let mut to_grass = Vec::new();
        let mut to_dry = Vec::new();
        let res = mask::RES as f32;
        let (mw, mh) = (m.rooms.w, m.rooms.h);
        let (mut to_rough, mut to_meadow) = (Vec::new(), Vec::new());
        // Доля пола в точке маски. Пятна по снимку ложатся по ней, а не по клетке
        // с соседями: у края контура OSM клеточная проверка резала пятно по сетке —
        // ступеньки в клетку на краю снега у ледника Мёнхсйоха. К краю (доля
        // 0,95 → 0,6) пятно слабеет и на соседний пол не лезет.
        let ice_mat = (!on_ice.is_empty()).then(|| floors.get(&mut m, "ice_glacier"));
        let share_of = |mat: usize| -> Vec<f32> {
            match m.terrain.iter().find(|c| c.material as usize == mat) {
                Some(c) => c.mask.data.iter().map(|&v| v as f32 / 255.0).collect(),
                None => vec![0.0; mw * mh],
            }
        };
        let (base_share, rough_share) = (share_of(base), share_of(rough));
        let ice_share = ice_mat.map_or_else(Vec::new, share_of);
        let fade = |x: f32| {
            let t = ((x - 0.6) / 0.35).clamp(0.0, 1.0);
            t * t * (3.0 - 2.0 * t)
        };
        for j in 0..mh {
            for i in 0..mw {
                let p = [(i as f32 + 0.5) / res, (j as f32 + 0.5) / res];
                let cell = (p[1] as usize).min(h as usize - 1) * w as usize + (p[0] as usize).min(w as usize - 1);
                let k = j * mw + i;
                let marks = [to_earth.len(), to_snow.len(), to_rock.len(), to_ice.len(), to_grass.len(), to_dry.len(), to_rough.len(), to_meadow.len()];
                let mut f = 1.0f32;
                // Со снимком пятна — где они на самом деле: бурое на лугу — орляк и
                // сухая трава, зелень на пустоши и на камне — трава. Без снимка — по шуму.
                let seen = |class: Class| hint.as_ref().map(|h| h.share(p, 0.8, class));
                let ramp = |x: f32, a: f32, b: f32| {
                    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
                    t * t * (3.0 - 2.0 * t)
                };
                if rock_here.get(k).copied().unwrap_or(on_rock[cell]) {
                    // Пятна не мельче нескольких клеток: в клетку-две они упирались
                    // в шаг маски (4 точки на клетку) и выходили квадратными кляксами.
                    let v = match seen(Class::Rock) {
                        Some(rock) => ramp(1.0 - rock - seen(Class::Water).unwrap_or(0.0), 0.5, 0.85),
                        None => {
                            let n = noise(p, 10.0, 25) * 0.72 + noise(p, 3.8, 26) * 0.2 + noise(p, 1.3, 29) * 0.08;
                            ((n - 0.18) / 0.32).clamp(0.0, 1.0) * 0.85
                        }
                    };
                    if v > 0.0 {
                        to_grass.push((k, v));
                    }
                } else if base_share[k] > 0.6 {
                    f = fade(base_share[k]);
                    // Вне зелёного биома основа — не луг: зелень на ней — луг, серое
                    // без тёплого оттенка — камень.
                    if base_slot != "meadow" {
                        if let Some(grass) = seen(Class::Grass) {
                            let v = ramp(grass, 0.3, 0.65) * 0.85;
                            if v > 0.0 {
                                to_meadow.push((k, v));
                            }
                        }
                        // Камень — где его почти сплошь: светлые гребни мергеля и
                        // дороги Табернаса при пороге 0,35 ложились пятнами по всей земле.
                        if let Some(rock) = seen(Class::Rock) {
                            let v = ramp(rock, 0.5, 0.85) * 0.85;
                            if v > 0.0 {
                                to_rock.push((k, v));
                            }
                        }
                    }
                    for (class, slot_is_base, out) in [(Class::Earth, base_slot == "earth_pale", &mut to_earth), (Class::Snow, base_slot == "snow", &mut to_snow), (Class::Ice, base_slot == "ice_glacier", &mut to_ice)] {
                        if let Some(x) = seen(class).filter(|_| !slot_is_base) {
                            let v = ramp(x, 0.3, 0.65) * 0.85;
                            if v > 0.0 {
                                out.push((k, v));
                            }
                        }
                    }
                    // Мелкая октава в клетку — край пятна рваный, как у травы, а не
                    // ломаный по отрезкам сетки шума.
                    // Сухая и скошенная трава — своим ровным полом, не кочками пустоши.
                    if let Some(dry) = seen(Class::Dry).filter(|_| base_slot != "straw_grass") {
                        let v = ramp(dry, 0.3, 0.65) * 0.85;
                        if v > 0.0 {
                            to_dry.push((k, v));
                        }
                    }
                    let v = match seen(Class::Brown) {
                        Some(brown) => ramp(brown, 0.3, 0.65) * 0.8,
                        None => {
                            let n = noise(p, 13.0, 21) * 0.62 + noise(p, 4.5, 22) * 0.26 + noise(p, 1.2, 27) * 0.12;
                            ((n - 0.2) / 0.3).clamp(0.0, 1.0) * 0.65
                        }
                    };
                    if v > 0.0 {
                        to_rough.push((k, v));
                    }
                } else if ice_share.get(k).is_some_and(|&x| x > 0.6) {
                    f = fade(ice_share[k]);
                    // Морена на леднике — камень по снимку; фирн (белое) — снег.
                    if let Some(rock) = seen(Class::Rock) {
                        let v = ramp(rock, 0.35, 0.7) * 0.85;
                        if v > 0.0 {
                            to_rock.push((k, v));
                        }
                    }
                    if let Some(snow) = seen(Class::Snow) {
                        let v = ramp(snow, 0.3, 0.65) * 0.9;
                        if v > 0.0 {
                            to_snow.push((k, v));
                        }
                    }
                } else if rough_share[k] > 0.6 {
                    f = fade(rough_share[k]);
                    // Кустарник в пустыне — по голой земле; снег на пустоши — снег.
                    for (class, out) in [(Class::Earth, &mut to_earth), (Class::Snow, &mut to_snow)] {
                        if let Some(x) = seen(class) {
                            let v = ramp(x, 0.3, 0.65) * 0.85;
                            if v > 0.0 {
                                out.push((k, v));
                            }
                        }
                    }
                    // Пустошь, что на снимке — светлый выгон (пастбища Малхема под тем же тегом).
                    if let Some(dry) = seen(Class::Dry) {
                        let v = ramp(dry, 0.3, 0.65) * 0.85;
                        if v > 0.0 {
                            to_dry.push((k, v));
                        }
                    }
                    let v = match seen(Class::Grass) {
                        Some(grass) => ramp(grass, 0.45, 0.8) * 0.75,
                        None => {
                            let n = noise(p, 9.0, 23) * 0.62 + noise(p, 3.5, 24) * 0.26 + noise(p, 1.2, 28) * 0.12;
                            ((n - 0.15) / 0.3).clamp(0.0, 1.0) * 0.6
                        }
                    };
                    if v > 0.0 {
                        to_meadow.push((k, v));
                    }
                }
                if f < 1.0 {
                    for (out, from) in [&mut to_earth, &mut to_snow, &mut to_rock, &mut to_ice, &mut to_grass, &mut to_dry, &mut to_rough, &mut to_meadow].into_iter().zip(marks) {
                        for e in &mut out[from..] {
                            e.1 *= f;
                        }
                    }
                }
            }
        }
        // На болоте тёмное по снимку — кочки: тёмный мох (бурая трава FA светлее
        // мха и выходила светлыми пятнами).
        let hummock = if bog { floors.get(&mut m, "bog_hummock") } else { rough };
        m.paint(Layer::Floor, hummock, &to_rough);
        m.paint(Layer::Floor, meadow, &to_meadow);
        // Трава на камне, что по снимку зарос, — основа: в пустыне земля, на леднике снег.
        m.paint(Layer::Floor, base, &to_grass);
        if !to_dry.is_empty() {
            // Выгоревший мох болота — светлая сухая трава вполсилы: сплошь мох FA
            // темнее снимка, а без светлых пятен болото — ровный бурый ковёр.
            if bog {
                let straw = floors.get(&mut m, "straw_grass");
                let light: Vec<(usize, f32)> = to_dry.iter().map(|&(k, v)| (k, v * 0.6)).collect();
                m.paint(Layer::Floor, straw, &light);
            } else {
                let dry = floors.get(&mut m, "dry_grass");
                m.paint(Layer::Floor, dry, &to_dry);
            }
        }
        // Голая земля: на сухом пастбище — бурая потрескавшаяся, в пустыне — светлая.
        // Камень и земля на леднике — серая морена (сланец): мел и мергель на нём
        // выходили бежевыми.
        for (slot, cells) in [(if cold { "rock_alpine" } else if base_slot == "straw_grass" { "earth_dry" } else { "earth_pale" }, &to_earth), ("ice_glacier", &to_ice), ("snow", &to_snow), (if cold { "rock_alpine" } else { rock_kind(&Tags::new()).0 }, &to_rock)] {
            if !cells.is_empty() {
                let t = floors.get(&mut m, slot);
                m.paint(Layer::Floor, t, cells);
            }
        }
    }

    // Верховое болото — топь целиком: пятна выгоревшего мха, зелени и кочек тоже
    // болото для «Живой карты» и для шага (вдвое дороже), а не только сам мох.
    if bog {
        for slot in ["bog_moss", "bog_hummock", "straw_grass", "meadow"] {
            if let Some(&i) = floors.0.get(slot) {
                m.materials[i].live = Live::Swamp;
            }
        }
    }

    // Дома: комнаты по контуру.
    let mut houses: Vec<House> = Vec::new();
    let mut ramparts: Vec<(Vec<Vec<[f32; 2]>>, WallStyle)> = Vec::new();
    let mut names: BTreeMap<String, ([f32; 2], f32)> = BTreeMap::new();
    for a in &areas {
        let Some(kind) = building(a.tags) else { continue };
        let weights = coverage(&m.rooms, &a.rings, 0.0);
        if weights.is_empty() {
            continue;
        }
        let (floor, slot, wood) = building_look(kind, a.tags);
        // Крепость — кирпич в своём цвете: стена — по цвету верха (тег, а нет —
        // снимок), боевая башня — по цвету стен (сверху у неё крыша). Кремлёвская
        // стена выходила домом с дощатым полом, хотя в OSM она `#ec8859`.
        let wall_top = rampart(a.tags);
        let fort = wall_top || get(a.tags, "tower:type") == "defensive";
        let tag = |k: &str| colour(get(a.tags, k));
        let tint = if wall_top {
            tag("roof:colour").or_else(|| tag("building:colour")).or_else(|| hint.as_ref().and_then(|h| h.colour(&weights, m.rooms.w)))
        } else if fort {
            tag("building:colour")
        } else {
            None
        };
        let f = match (fort, tint) {
            (true, Some(c)) => floors.tinted(&mut m, "brick_floor", c),
            (true, None) => floors.get(&mut m, "brick_floor"),
            _ => floors.get(&mut m, floor),
        };
        m.paint(Layer::Rooms, f, &weights);
        if wall_top {
            let teeth = [0, 1, 2].map(|i| (tint.unwrap_or([150, 140, 125])[i] as f32 * 0.72) as u8);
            ramparts.push((a.rings.clone(), WallStyle { width: 0.4, ..wall_tinted(slot, Some(teeth)) }));
        }
        let entrances = a
            .nodes
            .iter()
            .flatten()
            .filter_map(|id| osm.nodes.get(id))
            .filter(|n| !matches!(get(&n.tags, "entrance"), "" | "no") || !get(&n.tags, "door").is_empty())
            .map(|n| proj.at(n.lat, n.lon))
            .collect();
        let grand = matches!(kind, "train_station" | "transportation" | "church" | "cathedral" | "civic" | "public" | "commercial" | "retail" | "hotel" | "government");
        let (c, cells) = centre(&m.rooms, &weights);
        let name = get(a.tags, "name");
        if !name.is_empty() && cells >= 40.0 && names.get(name).is_none_or(|n| n.1 < cells) {
            names.insert(name.to_owned(), (c, cells));
        }
        let style = if fort { wall_tinted(slot, tag("building:colour").or(tint)) } else { wall(slot) };
        houses.push(House { rings: a.rings.clone(), entrances, style, wood: wood && !fort, grand, doors: !wall_top });
    }
    // Зубцы — когда все дома на месте: за краем, где башня, их не ставить.
    for (rings, teeth) in &ramparts {
        battlements(&mut m, rings, teeth);
    }
    m.prune();
    // Названия озёр, лесов, пустошей.
    for a in &areas {
        let name = get(a.tags, "name");
        if name.is_empty() || building(a.tags).is_some() || (ground(a.tags).is_none() && growth(a.tags).is_none()) {
            continue;
        }
        let (c, cells) = centre(&m.rooms, &coverage(&m.rooms, &a.rings, 0.0));
        if cells >= 60.0 && names.get(name).is_none_or(|n| n.1 < cells) {
            names.insert(name.to_owned(), (c, cells));
        }
    }

    // Линии снизу вверх: вода, рельсы, тропинки, улицы; мосты — поверх.
    let mut lanes: Vec<Lane> = Vec::new();
    // Проезжие улицы и пешеходные дорожки — порознь: дверь дома ищет сначала улицу.
    let mut streets: Vec<Vec<[f32; 2]>> = Vec::new();
    let mut walks: Vec<Vec<[f32; 2]>> = Vec::new();
    let mut street_names: BTreeMap<String, ([f32; 2], [f32; 2], f32)> = BTreeMap::new();
    let mut stones: Vec<[f32; 2]> = Vec::new();
    let rails_mat = m.material_index(Material { name: "Рельсы".into(), paint: Paint::Color([84, 82, 80]), vary: false, live: Live::Off }).unwrap_or(0);
    let deck_base = m.material_index(Material { name: "Настил".into(), paint: Paint::Color([70, 50, 34]), vary: false, live: Live::Off }).unwrap_or(0);
    // Тропа по пляжу и гальке не видна: на снимке Лулворта следа нет, а на карте
    // береговая тропа ложилась вдоль пляжа тёмной полосой.
    let beaches: Vec<&Area> = areas.iter().filter(|a| matches!(get(a.tags, "natural"), "beach" | "shingle" | "sand")).collect();
    let on_beach = |p: [f32; 2]| beaches.iter().any(|b| inside(&b.rings, p));
    // Пешеходная линия по площади — разметка для маршрутов: по Красной площади
    // они ложились светлыми полосами поперёк сплошной брусчатки.
    let plazas: Vec<&Area> = areas.iter().filter(|a| plaza(a.tags) && building(a.tags).is_none()).collect();
    let on_plaza = |p: [f32; 2]| plazas.iter().any(|b| inside(&b.rings, p));
    for way in osm.ways.values() {
        let t = &way.tags;
        if t.is_empty() || unseen(t) {
            continue;
        }
        let pts: Vec<[f32; 2]> = way.nodes.iter().filter_map(|id| osm.nodes.get(id)).map(|n| proj.at(n.lat, n.lon)).collect();
        let closed = way.nodes.len() >= 4 && way.nodes.first() == way.nodes.last();
        let lay = layer(t);
        for line in near_map(&pts, w as f32, h as f32, 12.0) {
            let line = dense(&line, 2.0);
            let path = |material: usize, width: f32, soft: f32, rough: f32| Path { points: line.clone(), material, width, soft, rough, along: false };
            if let Some((metres, deep)) = stream(t) {
                let water = floors.get(&mut m, if deep { "water_deep" } else { "water_shallow" });
                lanes.push((lay, 0, vec![path(water, m_to_cells(metres), 0.3, 0.15)]));
            } else if let Some((gauge, ballast)) = rail(t) {
                let half = m_to_cells(gauge / 2.0 + 0.035);
                let mut v = Vec::new();
                if ballast {
                    let gravel = floors.get(&mut m, "gravel");
                    v.push(path(gravel, m_to_cells(gauge + 1.8), 0.25, 0.1));
                }
                for side in [-half, half] {
                    v.push(Path { points: offset(&line, side), material: rails_mat, width: 0.1, soft: 0.0, rough: 0.0, along: false });
                }
                lanes.push((lay, if ballast { 1 } else { 5 }, v));
            } else if get(t, "ford") == "stepping_stones" || get(t, "surface") == "stepping_stones" {
                // Камни-переходы: цепочка камней через воду с ровным шагом по
                // всей длине, полосы дороги нет.
                let mut next = 0.0;
                let mut walked = 0.0;
                for s in line.windows(2) {
                    let l = length(s[0], s[1]);
                    while next <= walked + l {
                        let f = (next - walked) / l.max(1e-6);
                        stones.push([s[0][0] + (s[1][0] - s[0][0]) * f, s[0][1] + (s[1][1] - s[0][1]) * f]);
                        next += STEPPING;
                    }
                    walked += l;
                }
            } else if let Some((metres, tex, order)) = road(t, rural) {
                // Мост: у пешеходного — дощатый настил вдоль и деревянные перила,
                // у проезжего — своё мощение и каменный парапет.
                let bridge = !matches!(get(t, "bridge"), "" | "no");
                let tex = if bridge && order == 3 {
                    "wood_planks"
                } else if cold && natural_surface(tex) {
                    "snow_trampled"
                } else {
                    tex
                };
                let material = floors.get(&mut m, tex);
                // Гравийная дорожка парка ровнее тропы: на снимке у неё чёткий край.
                let (soft, rough) = if bridge {
                    (0.0, 0.0)
                } else if tex == "path_gravel" {
                    (0.3, 0.12)
                } else if natural_surface(tex) {
                    (0.45, 0.25)
                } else {
                    (0.12, 0.04)
                };
                let width = m_to_cells(metres).max(1.0);
                // Рыхлое покрытие или приливная — по пляжу следа нет (`surface=pebblestone`,
                // `tidal=yes` у береговой тропы Лулворта). Мощёная — видна.
                let loose = matches!(get(t, "surface"), "" | "pebblestone" | "gravel" | "fine_gravel" | "sand" | "shingle" | "ground" | "dirt" | "earth" | "grass" | "rock" | "unpaved" | "compacted")
                    || get(t, "tidal") == "yes";
                // Ступени видны и на площади: это перепад, а не разметка.
                let steps = get(t, "highway") == "steps";
                let hidden = |p: [f32; 2]| (loose && on_beach(p)) || (!steps && on_plaza(p));
                if order == 3 && !bridge && line.iter().any(|&p| hidden(p)) {
                    // Куски тропы вне пляжа и площади — порознь, по ним — ничего.
                    let mut run: Vec<[f32; 2]> = Vec::new();
                    for &p in line.iter().chain(std::iter::once(&[f32::NAN; 2])) {
                        if p[0].is_finite() && !hidden(p) {
                            run.push(p);
                            continue;
                        }
                        if run.len() >= 2 {
                            let points = std::mem::take(&mut run);
                            lanes.push((lay, order, vec![Path { points, material, width, soft, rough, along: false }]));
                        }
                        run.clear();
                    }
                    // Для дверей домов тропа остаётся тропой и там, где её не видно.
                    walks.push(line.clone());
                    continue;
                }
                let mut deck = path(material, width, soft, rough);
                deck.along = bridge && order == 3;
                // Доски FA полупрозрачны (между ними щели) — под настилом сплошная
                // подложка цвета дерева, иначе сквозь мост видно реку.
                let under = (bridge && order == 3).then(|| path(deck_base, width, 0.0, 0.0));
                lanes.push((lay, order, under.into_iter().chain([deck]).collect()));
                // Настил по болоту (`bridge=boardwalk`) лежит на земле, перил у него
                // нет: на Виру раба с тропы нельзя было сойти в болото. Перила моста —
                // только у карты: за краем сцены они тянулись на десятки клеток.
                if bridge && get(t, "bridge") != "boardwalk" {
                    let style = WallStyle { width: 0.15, ..wall(if order == 3 { "wall_wood" } else { "wall_stone_blocks" }) };
                    for side in [-width / 2.0, width / 2.0] {
                        for points in near_map(&offset(&line, side), w as f32, h as f32, 0.5) {
                            m.walls.push(Wall { points, closed: false, bends: Vec::new(), low: true, style: Some(style.clone()) });
                        }
                    }
                }
                (if order >= 4 { &mut streets } else { &mut walks }).push(line.clone());
                let name = get(t, "name");
                if order >= 4
                    && !name.is_empty()
                    && let Some((a, b)) = longest(&line)
                    && street_names.get(name).is_none_or(|s| s.2 < length(a, b))
                {
                    street_names.insert(name.to_owned(), (a, b, m_to_cells(metres)));
                }
            } else if get(t, "man_made") == "pier" && !closed {
                // Причал линией — дощатый настил вдоль, с подложкой, как у мостков.
                let width = m_to_cells(width_tag(t).unwrap_or(3.0)).max(1.0);
                let planks = floors.get(&mut m, "wood_planks");
                let mut deck = path(planks, width, 0.0, 0.0);
                deck.along = true;
                lanes.push((lay.max(1.0), 3, vec![path(deck_base, width, 0.0, 0.0), deck]));
                walks.push(line.clone());
            } else if get(t, "railway") == "platform" && !closed {
                let tiles = floors.get(&mut m, "stone_tiles");
                lanes.push((lay, 2, vec![path(tiles, m_to_cells(width_tag(t).unwrap_or(4.0)), 0.05, 0.0)]));
            }
        }
    }
    lanes.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    m.paths.extend(lanes.into_iter().flat_map(|l| l.2));

    walls_and_doors(&mut m, &houses, &streets, &walks);

    // Ограды: стены, заборы, решётки, изгороди, обрывы; у калиток и ворот — проход.
    for way in osm.ways.values() {
        let Some((slot, low, width)) = barrier(&way.tags, rural) else { continue };
        if unseen(&way.tags) {
            continue;
        }
        let nodes: Vec<&Node> = way.nodes.iter().filter_map(|id| osm.nodes.get(id)).collect();
        let pts: Vec<[f32; 2]> = nodes.iter().map(|n| proj.at(n.lat, n.lon)).collect();
        let gaps: Vec<Option<f32>> = nodes.iter().map(|n| gate(&n.tags)).collect();
        let style = WallStyle { width, ..wall(slot) };
        for piece in split_at_gates(&pts, &gaps) {
            for line in near_map(&piece, w as f32, h as f32, 0.5) {
                m.walls.push(Wall { points: line, closed: false, bends: Vec::new(), low, style: Some(style.clone()) });
            }
        }
    }

    // Обрывы — рельефом: земля опускается с нижней стороны линии, полосы скалы,
    // тени и стены Foundry строит рельеф.
    let cliffs: Vec<(Vec<[f32; 2]>, f32)> = osm
        .ways
        .values()
        .filter(|way| get(&way.tags, "natural") == "cliff" && !unseen(&way.tags))
        .flat_map(|way| {
            let pts: Vec<[f32; 2]> = way.nodes.iter().filter_map(|id| osm.nodes.get(id)).map(|n| proj.at(n.lat, n.lon)).collect();
            let levels = cliff_levels(&way.tags);
            near_map(&pts, w as f32, h as f32, 25.0).into_iter().map(move |l| (l, levels))
        })
        .collect();
    if let Some(mut r) = cliff_relief(&cliffs, w, h) {
        relief_strips(&mut r, Season::Summer);
        r.quantize();
        m.relief = Some(r);
    }

    // Лес, сад, кустарник, вереск, болото: отдельных деревьев в OSM у них нет —
    // сажаем дротиками, мимо домов, дорог, воды, оград и соседей.
    let mut blocked = Blocked::new(&m);
    for a in &areas {
        if matches!(ground(a.tags), Some(("water_deep" | "water_shallow", _))) {
            for (k, v) in coverage(&m.rooms, &a.rings, 0.0) {
                if v > 0.5 {
                    blocked.cells[k] = true;
                }
            }
        }
    }
    // Дома и вода: над ними кроны не нависают, над дорогой и оградой — да.
    let hard = blocked.clone();
    // Полоса скалы ложится вниз от кромки, до трёх клеток: на ней не растёт.
    for (l, _) in &cliffs {
        blocked.line(l, 1.0);
        blocked.line(&offset(l, -1.8), 1.8);
    }
    for p in &m.paths {
        blocked.line(&p.curve(), p.width / 2.0 + 0.6);
    }
    for wl in &m.walls {
        blocked.line(&wl.points, 0.8);
    }
    let mut spacing = Spacing::default();
    let mut planted = 0usize;
    let mut litter: Vec<([f32; 2], f32)> = Vec::new();

    // Кладбище: могилы рядами, изголовьем на запад, как у церкви генератора; мимо
    // дорожек, оград и домов. Без надгробий в паках — холмик земли с камнем.
    let tombstones = !missing("tombstone");
    for (k, a) in areas.iter().enumerate().filter(|(_, a)| get(a.tags, "amenity") == "grave_yard" || get(a.tags, "landuse") == "cemetery") {
        let Some((lo, hi)) = clipped_box(&a.rings, w as f32, h as f32) else { continue };
        let dirt = floors.get(&mut m, "dirt");
        let mut rng = Rng::new(&format!("osm/graves/{k}"));
        let mut y = lo[1].floor() + 0.5;
        let mut row = 0;
        while y + 1.0 < hi[1] {
            let mut x = lo[0].floor() + 0.5 + (row % 2) as f32 * 1.5;
            while x + 2.0 < hi[0] {
                let q = [x + 1.0, y + 0.5];
                let spots = [[x, y], [x + 2.0, y], [x, y + 1.0], [x + 2.0, y + 1.0], q];
                if spots.iter().all(|&c| !blocked.at(c) && inside(&a.rings, c)) && rng.chance(0.55) {
                    church::grave(&mut m, &mut rng, q, tombstones, dirt, false);
                    blocked.disk(q, 1.2);
                }
                x += 3.0;
            }
            y += 2.5;
            row += 1;
        }
    }

    // Деревья по снимку — первыми, по дереву на крону и размером с неё (см.
    // [`Hint::crowns`]): в лесу и за его краем, на лугу и у реки. В OSM одиноких
    // деревьев и редколесья по склонам почти нет. В городе тёмное на снимке — и
    // тени домов, так что там кроны встают только на зелёном OSM (лес, парк,
    // газон) и не на мощёном. Без них в городском лесу не росло ни дерева: лес со
    // снимком деревьев не сажает, ждёт крон (Тайницкий сад Кремля был газоном).
    let mut planted_crowns: Vec<([f32; 2], f32)> = Vec::new();
    let town_green: Option<Vec<bool>> = (!rural && hint.is_some()).then(|| {
        let mut green = vec![false; m.rooms.w * m.rooms.h];
        let leafy = |t: &Tags| growth(t).is_some() || matches!(ground(t), Some((g, _)) if matches!(g, "meadow" | "rough_grass" | "straw_grass"));
        let hard = |t: &Tags| plaza(t) || building(t).is_some() || matches!(ground(t), Some((g, _)) if matches!(g, "stone_tiles" | "street" | "flagstones"));
        for (on, a) in areas.iter().filter_map(|a| if leafy(a.tags) { Some((true, a)) } else { None }).chain(areas.iter().filter(|a| hard(a.tags)).map(|a| (false, a))) {
            for (k, v) in coverage(&m.rooms, &a.rings, 0.0) {
                if v > 0.5 {
                    green[k] = on;
                }
            }
        }
        green
    });
    if let Some(hint) = hint.as_ref() {
        let mut rng = Rng::new("osm/cover-trees");
        let woods: Vec<&Area> = areas.iter().filter(|a| growth(a.tags).is_some_and(|g| g.litter)).collect();
        let orchards: Vec<&Area> = areas.iter().filter(|a| get(a.tags, "landuse") == "orchard").collect();
        let mut crowns: Vec<Spot> = Vec::new();
        let res = mask::RES as f32;
        for Spot { q, mut r, conifer, queue } in hint.trees(w as usize, h as usize, cell) {
            if blocked.at(q) || hint.share(q, r.min(2.0), Class::Water) > 0.3 {
                continue;
            }
            if let Some(green) = &town_green
                && !green[(q[1] * res) as usize * m.rooms.w + (q[0] * res) as usize]
            {
                continue;
            }
            // Крона нависает над дорогой и оградой (на снимке полог над дорогой
            // смыкается), над домом и водой — не дальше половины радиуса.
            let hits = |r: f32| (0..8).any(|i| hard.at([q[0] + 0.5 * r * (i as f32 * FRAC_PI_4).cos(), q[1] + 0.5 * r * (i as f32 * FRAC_PI_4).sin()]));
            while r >= CROWN_MIN && hits(r) {
                r *= 0.85;
            }
            if r >= CROWN_MIN {
                crowns.push(Spot { q, r, conifer, queue });
            }
        }
        // Мелкие снизу: большое дерево выше и ложится поверх.
        crowns.sort_by(|a, b| a.r.total_cmp(&b.r));
        for Spot { q, r, conifer, queue } in crowns {
            if planted >= MAX_PLANTS {
                break;
            }
            // Порода: по карте лесов, если анализ снимка её знал; иначе в лесу OSM —
            // по его `leaf_type`, долей хвойных из очереди (куртинами, тёмные первыми);
            // за лесом — лиственное. Ели наугад вставали тёмными звёздами в ясеневой
            // роще Малхема и по одной среди дубов Болтона.
            let wood = woods.iter().find(|a| inside(&a.rings, q)).and_then(|a| growth(a.tags));
            // Сад — оливы: в сухом краю всякий, в зелёном — по тегу (`trees=olive_trees`).
            let olive = orchards.iter().any(|a| inside(&a.rings, q) && (arid || get(a.tags, "trees").contains("olive") || get(a.tags, "crop") == "olive"));
            let slot = if olive && r >= CROWN_BUSH { "olivo" } else { crown_slot(r, conifer, wood.as_ref(), queue, arid) };
            if missing(slot) {
                continue;
            }
            drop_crown(&mut m, slot, q, rng.int(0, 359) as f32, r);
            planted += 1;
            planted_crowns.push((q, r));
            spacing.add(q);
            // Под кроной ничего не сажать: мелочь легла бы поверх неё.
            blocked.disk(q, r * 0.95);
            if r >= CROWN_BUSH && wood.is_some() {
                litter.push((q, r));
            }
        }
    }

    let mut sown: Vec<(u8, usize, Growth)> = areas.iter().enumerate().filter_map(|(k, a)| growth(a.tags).map(|g| (g.rank, k, g))).collect();
    sown.sort_by_key(|s| (s.0, s.1));
    for (n, (rank, k, g)) in sown.iter().enumerate() {
        let a = &areas[*k];
        let (k, rank) = (*k, *rank);
        // Старшие контуры, что могут лежать поверх этого.
        let above: Vec<&Area> = sown[..n].iter().filter(|s| s.0 < rank).map(|s| &areas[s.1]).collect();
        let Some((lo, hi)) = clipped_box(&a.rings, w as f32, h as f32) else { continue };
        let mut rng = Rng::new(&format!("osm/plant/{k}"));
        let tries = ((hi[0] - lo[0]) * (hi[1] - lo[1]) * 1.5) as usize;
        for _ in 0..tries {
            if planted >= MAX_PLANTS {
                break;
            }
            let q = [lo[0] + rng.random() as f32 * (hi[0] - lo[0]), lo[1] + rng.random() as f32 * (hi[1] - lo[1])];
            if blocked.at(q) {
                continue;
            }
            // Лес местами гуще и реже, с полянами в папоротнике: на снимке ровной
            // сетки крон не бывает. Пустошь и кустарник — куртинами. Со снимком
            // поляна — где на нём просвет.
            let glade = g.litter
                && match &hint {
                    Some(h) => h.share(q, 1.5, Class::Trees) < 0.3,
                    None => noise(q, 9.0, 8) > 0.45,
                };
            let local = if glade {
                2.2
            } else if g.litter {
                g.spacing * (1.0 + 0.3 * noise(q, 14.0, 7))
            } else {
                g.spacing
            };
            if !spacing.free(q, local) || !inside(&a.rings, q) || above.iter().any(|b| inside(&b.rings, q)) {
                continue;
            }
            spacing.add(q);
            let share = if glade {
                0.35
            } else if g.litter {
                g.share
            } else if g.rank >= 3 && g.rank <= 4 {
                // Пустошь и болото — куртинами между открытыми местами: ровный
                // «горох» кустов по всей пустоши против снимка выглядел узором.
                // Со снимком — где на нём бурое (орляк, вереск); деревья уже стоят.
                let c = match &hint {
                    Some(h) => ((h.share(q, 1.2, Class::Brown) - 0.25) / 0.4).clamp(0.0, 1.0),
                    None => ((noise(q, 11.0, 9) + 0.1) / 0.45).clamp(0.0, 1.0),
                };
                (g.share * 1.9 * (c * c * (3.0 - 2.0 * c)) as f64).clamp(0.0, 0.95)
            } else {
                (g.share * (1.0 + 0.8 * noise(q, 10.0, 9) as f64)).clamp(0.0, 1.0)
            };
            if !rng.chance(share) {
                continue;
            }
            let slot = arid_slot(
                if glade {
                    if rng.chance(0.7) { "fern" } else { "bush_summer" }
                } else {
                    g.slots[rng.int(0, g.slots.len() as i32 - 1) as usize]
                },
                arid,
            );
            // Со снимком деревья уже стоят по его кронам: лишние мелкие у большой
            // кроны дробили её.
            if hint.is_some() && (slot.starts_with("tree") || slot == "pine") {
                continue;
            }
            if missing(slot) {
                continue;
            }
            let scale = g.scale.0 + rng.random() as f32 * (g.scale.1 - g.scale.0);
            if slot == "boulder" {
                // Камни одной породы: вперемешку с синими и рудными выходила пёстрая
                // галька. На пустоши — мшистые, на скале и осыпи — по тегу `rock`.
                let tone = if growth_rock(a.tags) { rock_kind(a.tags).1 } else { "mossy" };
                drop_pick(&mut m, &asset_toned("boulder", tone), q, rng.int(0, 359) as f32, scale);
            } else {
                drop(&mut m, slot, q, rng.int(0, 359) as f32, scale);
            }
            // Папоротник ложится в картинку карты, тайла у него нет — в предел не идёт.
            if !slot.starts_with("fern") {
                planted += 1;
            }
            if g.litter && (slot.starts_with("tree") || slot == "pine") {
                litter.push((q, 2.4));
            }
        }
    }

    // Ничья земля за городом — ни луга, ни поля, ни леса в OSM: выгон и неудобья.
    // Одинокие кусты и деревья, местами купами; голый ковёр травы против снимка
    // выглядел пустым. Луг, поле, парк и кладбище не трогаются.
    if rural {
        let mw = m.rooms.w;
        let mut claimed = vec![false; mw * m.rooms.h];
        for a in areas.iter().filter(|a| ground(a.tags).is_some() || growth(a.tags).is_some() || building(a.tags).is_some()) {
            for (k, v) in coverage(&m.rooms, &a.rings, 0.0) {
                if v > 0.0 {
                    claimed[k] = true;
                }
            }
        }
        let res = mask::RES as f32;
        let mut rng = Rng::new("osm/lone");
        for _ in 0..(w * h) as usize / 3 {
            if planted >= MAX_PLANTS {
                break;
            }
            let q = [rng.random() as f32 * w as f32, rng.random() as f32 * h as f32];
            if blocked.at(q) || claimed[(q[1] * res) as usize * mw + (q[0] * res) as usize] || !spacing.free(q, 4.5) {
                continue;
            }
            spacing.add(q);
            // Со снимком деревья и кусты уже стоят по его пятнам (куст — мелкое
            // пятно); здесь — редкие одиночные. Кусты по краю крон кольцом
            // обсаживали большое дерево мелочью.
            let edge = hint.as_ref().map(|h| h.share(q, 1.5, Class::Trees));
            let chance = match edge {
                Some(_) => 0.01,
                None => {
                    let copse = noise(q, 16.0, 11).max(0.0);
                    0.06 + 0.5 * copse * copse
                }
            };
            if !rng.chance(chance as f64) {
                continue;
            }
            let tree = edge.is_none() && rng.chance(0.45);
            let slot = if tree { "tree_green" } else { arid_slot("bush_summer", arid) };
            if missing(slot) {
                continue;
            }
            let scale = if tree { 1.0 + rng.random() as f32 * 0.5 } else { 0.7 + rng.random() as f32 * 0.5 };
            drop(&mut m, slot, q, rng.int(0, 359) as f32, scale);
            planted += 1;
        }
        // Верховое болото: на тёмных кочках (бурое по снимку) — кустарнички и
        // изредка низкая сосенка, на светлом мху — пучки осоки. Голый мох на
        // боевой карте читался ровным ковром, а на снимке он весь в кочках.
        if let Some(hint) = hint.as_ref().filter(|_| bog) {
            let mut rng = Rng::new("osm/bog");
            for _ in 0..(w * h) as usize {
                if planted >= MAX_PLANTS {
                    break;
                }
                let q = [rng.random() as f32 * w as f32, rng.random() as f32 * h as f32];
                if blocked.at(q) || claimed[(q[1] * res) as usize * mw + (q[0] * res) as usize] || !spacing.free(q, 1.6) {
                    continue;
                }
                let hummock = hint.share(q, 1.0, Class::Brown);
                if hint.share(q, 0.8, Class::Water) > 0.0 || !rng.chance((0.03 + 0.3 * hummock) as f64) {
                    continue;
                }
                let (slot, scale) = if hummock > 0.5 && rng.chance(0.12) {
                    ("fir", 0.55 + rng.random() as f32 * 0.25)
                } else if hummock > 0.3 {
                    ("bush_summer", 0.4 + rng.random() as f32 * 0.25)
                } else {
                    ("tussock", 0.5 + rng.random() as f32 * 0.3)
                };
                if missing(slot) {
                    continue;
                }
                spacing.add(q);
                drop(&mut m, slot, q, rng.int(0, 359) as f32, scale);
                planted += 1;
            }
        }
        // Рощи по желанию (`Options::trees`): куртины деревьев и кустов на свободной
        // суше — не на воде, пляже, камне, в домах и на тропах, не там, где снимок
        // видит воду, камень или снег. Порода — как у крон снимка (доля хвойных среди
        // крон с известной породой), в сухом краю — каменный дуб, на болоте — низкие
        // сосенки; на снегу и льду не растёт ничего.
        if opt.trees > 0.0 && !cold {
            let mut bare = vec![false; mw * m.rooms.h];
            for a in areas.iter() {
                let hard = building(a.tags).is_some()
                    || matches!(ground(a.tags), Some((t, _)) if t.starts_with("water") || t.starts_with("sea") || t.starts_with("rock_") || matches!(t, "sand" | "shingle" | "gravel" | "stone_tiles" | "street" | "flagstones"));
                if hard {
                    for (k, v) in coverage(&m.rooms, &a.rings, 0.0) {
                        bare[k] |= v > 0.3;
                    }
                }
            }
            let known: Vec<&Crown> = hint.as_ref().and_then(|h| h.cover.crowns.as_ref()).map(|c| c.iter().filter(|c| c.src != "none" && c.kind != "bush").collect()).unwrap_or_default();
            let needles = if known.len() >= 5 { known.iter().filter(|c| c.kind == "conifer").count() as f32 / known.len() as f32 } else { 0.15 };
            let mut rng = Rng::new("osm/grove");
            for _ in 0..(w * h) as usize {
                if planted >= MAX_PLANTS {
                    break;
                }
                let q = [rng.random() as f32 * w as f32, rng.random() as f32 * h as f32];
                if blocked.at(q) || bare[(q[1] * res) as usize * mw + (q[0] * res) as usize] {
                    continue;
                }
                // Галька у берега на снимке — «камень»: на ней не растёт, но лишь
                // где её больше половины; вода, снег и лёд — уже с четверти.
                if let Some(h) = &hint
                    && ([Class::Water, Class::Snow, Class::Ice].iter().any(|&c| h.share(q, 1.0, c) > 0.25) || h.share(q, 1.0, Class::Rock) > 0.5)
                {
                    continue;
                }
                let copse = (noise(q, 8.0, 41) * 0.5 + 0.5).clamp(0.0, 1.0);
                if !rng.chance((opt.trees * (0.03 + 0.5 * copse.powf(2.5) * 2.0)) as f64) {
                    continue;
                }
                // В гуще куртины — деревья, по её краю и вразброс — кусты.
                let tree = rng.chance((0.25 + 0.75 * copse) as f64);
                let (slot, scale, gap) = if !tree {
                    (if bog { "bush_summer" } else { arid_slot("bush_summer", arid) }, 0.6 + rng.random() as f32 * 0.4, 1.6)
                } else if bog {
                    ("fir", 0.55 + rng.random() as f32 * 0.25, 2.0)
                } else if arid {
                    ("encina", 0.9 + rng.random() as f32 * 0.5, 2.8)
                } else if rng.chance(needles as f64) {
                    let r = 1.6 + rng.random() as f32 * 1.0;
                    (conifer_slot(r), 0.8 + rng.random() as f32 * 0.4, 2.4)
                } else {
                    ("tree_green", 1.0 + rng.random() as f32 * 0.5, 2.8)
                };
                if missing(slot) || !spacing.free(q, gap) {
                    continue;
                }
                spacing.add(q);
                drop(&mut m, slot, q, rng.int(0, 359) as f32, scale);
                planted += 1;
                if tree && !bog {
                    litter.push((q, 2.4));
                }
            }
        }
    }

    // Кувшинки — купами у берега стоячей воды, где мелко, как в заводях на снимке.
    // Только из паков: встроенных нет. На реке их сносит течением.
    if !missing("lilypad") && !bog {
        let mut rng = Rng::new("osm/lily");
        for a in areas.iter().filter(|a| get(a.tags, "natural") == "water" && !matches!(get(a.tags, "water"), "river" | "canal" | "stream" | "lock" | "wastewater" | "fountain" | "sea" | "bay" | "lagoon")) {
            let Some((lo, hi)) = clipped_box(&a.rings, w as f32, h as f32) else { continue };
            let edge = |p: [f32; 2]| a.rings.iter().flat_map(|r| (0..r.len()).map(move |k| (r[k], r[(k + 1) % r.len()]))).map(|(u, v)| segment_distance(p, u, v)).fold(f32::INFINITY, f32::min);
            let cells = (hi[0] - lo[0]) * (hi[1] - lo[1]);
            let mut beds = (cells / 900.0).clamp(1.0, 10.0) as usize;
            for _ in 0..beds * 40 {
                if beds == 0 {
                    break;
                }
                let c = [lo[0] + rng.random() as f32 * (hi[0] - lo[0]), lo[1] + rng.random() as f32 * (hi[1] - lo[1])];
                if !inside(&a.rings, c) || !(1.0..2.6).contains(&edge(c)) {
                    continue;
                }
                beds -= 1;
                // Купа в 2–3 клетки, листья с клетку: мельче на карте их не видно.
                for _ in 0..rng.int(8, 16) {
                    let p = [c[0] + (rng.random() as f32 - 0.5) * 5.0, c[1] + (rng.random() as f32 - 0.5) * 5.0];
                    if inside(&a.rings, p) && edge(p) > 0.4 && m.contains(p[0].floor() as i32, p[1].floor() as i32) {
                        drop(&mut m, "lilypad", p, rng.int(0, 359) as f32, 0.8 + rng.random() as f32 * 0.5);
                    }
                }
            }
        }
    }
    if !litter.is_empty() {
        let f = floors.get(&mut m, "forest_floor");
        let weights = blobs(&m, &litter, 0.15, 0.8);
        m.paint(Layer::Floor, f, &weights);
    }
    if planted >= MAX_PLANTS {
        warnings.push(format!("деревьев и кустов больше {MAX_PLANTS}: часть леса не засажена"));
    }

    // Камни-переходы — одного вида на всю карту: у FA их пять пород, вперемешку пестро.
    let stone = asset("stepping_stone");
    for (k, &p) in stones.iter().enumerate() {
        if m.contains(p[0].floor() as i32, p[1].floor() as i32) {
            let mut rng = Rng::new(&format!("osm/stepping/{k}"));
            drop_pick(&mut m, &stone, p, rng.int(0, 359) as f32, 0.7);
        }
    }

    // Камни по точкам.
    let mut rocks: Vec<(i64, [f32; 2])> = osm.nodes.iter().filter(|(_, n)| matches!(get(&n.tags, "natural"), "rock" | "stone") && !unseen(&n.tags)).map(|(id, n)| (*id, proj.at(n.lat, n.lon))).collect();
    rocks.sort_by_key(|r| r.0);
    // Порода камней — как у скал на карте.
    let tone = areas.iter().find(|a| growth_rock(a.tags)).map_or("earthy", |a| rock_kind(a.tags).1);
    for (id, p) in rocks {
        if m.contains(p[0].floor() as i32, p[1].floor() as i32) {
            let mut rng = Rng::new(&format!("osm/rock/{id}"));
            let scale = 1.0 + rng.random() as f32 * 0.6;
            drop_pick(&mut m, &asset_toned("boulder", tone), p, rng.int(0, 359) as f32, scale);
        }
    }

    // Деревья: отдельные и рядами вдоль улиц.
    let mut trees: Vec<(i64, [f32; 2])> = osm.nodes.iter().filter(|(_, n)| get(&n.tags, "natural") == "tree" && !unseen(&n.tags)).map(|(id, n)| (*id, proj.at(n.lat, n.lon))).collect();
    for (id, way) in &osm.ways {
        if get(&way.tags, "natural") != "tree_row" {
            continue;
        }
        let pts: Vec<[f32; 2]> = way.nodes.iter().filter_map(|n| osm.nodes.get(n)).map(|n| proj.at(n.lat, n.lon)).collect();
        let step = m_to_cells(8.0);
        for (k, s) in pts.windows(2).enumerate() {
            let n = (length(s[0], s[1]) / step).floor() as usize;
            for q in 0..=n {
                let f = q as f32 / (n.max(1)) as f32;
                trees.push((id * 1000 + (k * 100 + q) as i64, [s[0][0] + (s[1][0] - s[0][0]) * f, s[0][1] + (s[1][1] - s[0][1]) * f]));
            }
        }
    }
    trees.sort_by_key(|t| t.0);
    let res = mask::RES as f32;
    let outdoors = |m: &Map, p: [f32; 2]| m.contains(p[0].floor() as i32, p[1].floor() as i32) && m.rooms.get((p[0] * res).floor() as isize, (p[1] * res).floor() as isize) < 128;
    for (id, p) in trees {
        // Дерево OSM в кроне со снимка — то же дерево: второе, мельче, вставало
        // рядом с ним или на нём.
        if !outdoors(&m, p) || planted_crowns.iter().any(|(c, r)| length(*c, p) < r.max(2.0)) {
            continue;
        }
        let mut rng = Rng::new(&format!("osm/tree/{id}"));
        let rot = rng.int(0, 359) as f32;
        let scale = 1.2 + rng.random() as f32 * 0.4;
        drop(&mut m, "tree_green", p, rot, scale);
    }

    // Уличное по точкам: фонари, скамьи (вдоль ближайшей улицы), фонтаны, памятники.
    let mut things: Vec<(i64, &'static str, [f32; 2])> = osm.nodes.iter().filter(|(_, n)| !unseen(&n.tags)).filter_map(|(id, n)| Some((*id, furniture(&n.tags)?, proj.at(n.lat, n.lon)))).collect();
    things.sort_by_key(|t| t.0);
    for (_, slot, p) in things {
        if !outdoors(&m, p) {
            continue;
        }
        let along = streets.iter().chain(&walks).flat_map(|l| l.windows(2)).min_by(|x, y| segment_distance(p, x[0], x[1]).total_cmp(&segment_distance(p, y[0], y[1])));
        let rot = match (slot, along) {
            ("bench", Some(s)) => (s[1][1] - s[0][1]).atan2(s[1][0] - s[0][0]).to_degrees().rem_euclid(180.0),
            _ => 0.0,
        };
        drop(&mut m, slot, p, rot, 1.0);
    }

    // Подписи для ведущего: улицы вдоль, большие здания посередине.
    for (name, (a, b, width)) in street_names {
        let mut l = Label::new(name, [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0]);
        l.size = (width * 0.4).clamp(0.6, 2.5);
        l.rot = upright(a, b);
        l.gm = true;
        m.labels.push(l);
    }
    for (name, (c, cells)) in names {
        let mut l = Label::new(name, c);
        l.size = (cells.sqrt() * 0.08).clamp(1.0, 3.0);
        l.gm = true;
        m.labels.push(l);
    }
    m.labels.retain(|l| l.pos[0] >= 0.0 && l.pos[1] >= 0.0 && l.pos[0] < w as f32 && l.pos[1] < h as f32);
    if houses.is_empty() && m.paths.is_empty() {
        warnings.push("в выгрузке нет ни домов, ни улиц".into());
    }
    Ok((m, Report { count: houses.len(), warnings }))
}

/// Как далеко от дома ещё ищется улица для двери; дальше — к дорожке.
const STREET_REACH: f32 = 12.0;

/// Стены домов: вид наружных по дому, общие стены соседей, двери.
fn walls_and_doors(m: &mut Map, houses: &[House], streets: &[Vec<[f32; 2]>], walks: &[Vec<[f32; 2]>]) {
    let res = mask::RES as f32;
    let (w, h) = (m.width as f32, m.height as f32);
    let edges = m.room_edges();
    let rooms = m.rooms.clone();
    let inside = |p: [f32; 2]| rooms.get((p[0] * res).floor() as isize, (p[1] * res).floor() as isize) >= 128;
    let on_border = |s: &Segment| {
        let edge = |v: f32, lim: f32| v.abs() < 0.05 || (v - lim).abs() < 0.05;
        (edge(s.0[0], w) && edge(s.1[0], w) && (s.0[0] - s.1[0]).abs() < 0.05) || (edge(s.0[1], h) && edge(s.1[1], h) && (s.0[1] - s.1[1]).abs() < 0.05)
    };
    let mut shared: BTreeSet<[i32; 4]> = BTreeSet::new();
    for house in houses {
        let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
        for p in house.rings.iter().flatten() {
            for c in 0..2 {
                lo[c] = lo[c].min(p[c]);
                hi[c] = hi[c].max(p[c]);
            }
        }
        let near_ring = |p: [f32; 2]| house.rings.iter().any(|r| (0..r.len()).any(|k| segment_distance(p, r[k], r[(k + 1) % r.len()]) < 0.25));
        let own: Vec<Segment> = edges
            .iter()
            .copied()
            .filter(|s| {
                let mid = [(s.0[0] + s.1[0]) / 2.0, (s.0[1] + s.1[1]) / 2.0];
                mid[0] >= lo[0] - 0.5 && mid[0] <= hi[0] + 0.5 && mid[1] >= lo[1] - 0.5 && mid[1] <= hi[1] + 0.5 && near_ring(mid)
            })
            .collect();
        m.set_edge_look(&own, &house.style);

        // Общая стена: по обе стороны отрезка контура дома — комнаты.
        for r in &house.rings {
            for k in 0..r.len() {
                let (a, b) = (r[k], r[(k + 1) % r.len()]);
                let len = length(a, b);
                let mid = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
                if len < 0.25 || !m.contains(mid[0].floor() as i32, mid[1].floor() as i32) {
                    continue;
                }
                let n = [-(b[1] - a[1]) / len * 0.5, (b[0] - a[0]) / len * 0.5];
                if !(inside([mid[0] + n[0], mid[1] + n[1]]) && inside([mid[0] - n[0], mid[1] - n[1]])) {
                    continue;
                }
                let q = |p: [f32; 2]| [(p[0] * 100.0).round() as i32, (p[1] * 100.0).round() as i32];
                let (qa, qb) = (q(a), q(b));
                let key = if qa <= qb { [qa[0], qa[1], qb[0], qb[1]] } else { [qb[0], qb[1], qa[0], qa[1]] };
                if shared.insert(key) {
                    m.walls.push(Wall { points: vec![a, b], closed: false, bends: Vec::new(), low: false, style: Some(house.style.clone()) });
                }
            }
        }

        // Двери: у отмеченных входов, иначе одна — к ближайшей улице.
        if !house.doors {
            continue;
        }
        let doorable: Vec<Segment> = own.iter().copied().filter(|s| length(s.0, s.1) >= 1.4 && !on_border(s)).collect();
        if doorable.is_empty() {
            continue;
        }
        let mut spots: Vec<[f32; 2]> = Vec::new();
        for &e in &house.entrances {
            if spots.len() < 6 && spots.iter().all(|s| length(*s, e) >= 2.5) {
                spots.push(e);
            }
        }
        if spots.is_empty() {
            let mid = |s: &Segment| [(s.0[0] + s.1[0]) / 2.0, (s.0[1] + s.1[1]) / 2.0];
            let dist = |lines: &[Vec<[f32; 2]>], p: [f32; 2]| lines.iter().flat_map(|l| l.windows(2)).map(|s| segment_distance(p, s[0], s[1])).fold(f32::INFINITY, f32::min);
            // Улица рядом — дверь к ней, а не во двор к дорожке; улицы нет — к дорожке.
            let near_street = doorable.iter().any(|s| dist(streets, mid(s)) < STREET_REACH);
            let lines = if near_street || walks.is_empty() { streets } else { walks };
            let best = doorable.iter().min_by(|x, y| dist(lines, mid(x)).total_cmp(&dist(lines, mid(y))).then(length(y.0, y.1).total_cmp(&length(x.0, x.1))));
            if let Some(s) = best {
                spots.push([(s.0[0] + s.1[0]) / 2.0, (s.0[1] + s.1[1]) / 2.0]);
            }
        }
        for p in spots {
            let Some(&(a, b)) = doorable.iter().min_by(|x, y| segment_distance(p, x.0, x.1).total_cmp(&segment_distance(p, y.0, y.1))) else { continue };
            if segment_distance(p, a, b) > 1.5 {
                continue;
            }
            let len = length(a, b);
            let width = if house.grand && len >= 3.4 { 2.0 } else { 1.0 };
            let dir = [(b[0] - a[0]) / len, (b[1] - a[1]) / len];
            let t = ((p[0] - a[0]) * dir[0] + (p[1] - a[1]) * dir[1]).clamp(width / 2.0 + 0.2, len - width / 2.0 - 0.2);
            let at = |s: f32| [a[0] + dir[0] * s, a[1] + dir[1] * s];
            let kind = if width >= 2.0 { PortalKind::DoubleDoor } else { PortalKind::Door };
            m.portals.push(portal(at(t - width / 2.0), at(t + width / 2.0), kind, house.wood));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Два дома стена к стене, улица с именем, рельсы, дерево, вход и подземная
    /// платформа, которой на карте быть не должно. Около 60×45 м.
    const SAMPLE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<osm version="0.6">
 <bounds minlat="51.5300" minlon="-0.1250" maxlat="51.5304" maxlon="-0.1241"/>
 <node id="1" lat="51.5303" lon="-0.1248"/>
 <node id="2" lat="51.5303" lon="-0.1245"/>
 <node id="3" lat="51.5302" lon="-0.1245"/>
 <node id="4" lat="51.5302" lon="-0.1248"/>
 <node id="5" lat="51.5303" lon="-0.1242"/>
 <node id="6" lat="51.5302" lon="-0.1242"/>
 <node id="7" lat="51.5302" lon="-0.12465"><tag k="entrance" v="main"/></node>
 <node id="10" lat="51.53013" lon="-0.1251"/>
 <node id="11" lat="51.53013" lon="-0.1240"/>
 <node id="20" lat="51.53005" lon="-0.1251"/>
 <node id="21" lat="51.53005" lon="-0.1240"/>
 <node id="30" lat="51.53035" lon="-0.12435"><tag k="natural" v="tree"/></node>
 <node id="40" lat="51.53030" lon="-0.1250"/>
 <node id="41" lat="51.53030" lon="-0.1241"/>
 <node id="50" lat="51.53009" lon="-0.1247"><tag k="highway" v="street_lamp"/></node>
 <node id="51" lat="51.53009" lon="-0.1244"><tag k="amenity" v="bench"/></node>
 <node id="60" lat="51.53037" lon="-0.1250"/>
 <node id="61" lat="51.53037" lon="-0.1246"/>
 <way id="500"><nd ref="60"/><nd ref="61"/><tag k="barrier" v="fence"/></way>
 <node id="70" lat="51.53033" lon="-0.1250"/>
 <node id="71" lat="51.53033" lon="-0.1241"/>
 <way id="600"><nd ref="70"/><nd ref="71"/><tag k="highway" v="footway"/></way>
 <way id="100"><nd ref="1"/><nd ref="2"/><nd ref="3"/><nd ref="7"/><nd ref="4"/><nd ref="1"/><tag k="building" v="house"/></way>
 <way id="101"><nd ref="2"/><nd ref="5"/><nd ref="6"/><nd ref="3"/><nd ref="2"/><tag k="building" v="retail"/><tag k="name" v="Lantern Shop"/></way>
 <way id="200"><nd ref="10"/><nd ref="11"/><tag k="highway" v="residential"/><tag k="name" v="Crowfall Lane"/></way>
 <way id="300"><nd ref="20"/><nd ref="21"/><tag k="railway" v="rail"/></way>
 <way id="400"><nd ref="40"/><nd ref="41"/><tag k="railway" v="platform"/><tag k="level" v="-1"/></way>
</osm>"#;

    fn sample() -> (Map, Report) {
        import(SAMPLE, Options::default()).expect("выгрузка читается")
    }

    #[test]
    fn houses_become_rooms_with_shared_wall_and_door_at_entrance() {
        let (m, r) = sample();
        assert_eq!(r.count, 2, "{:?}", r.warnings);
        assert!(r.warnings.is_empty(), "{:?}", r.warnings);
        // 0,0009° долготы на этой широте — около 62 м, 0,0004° широты — около 44 м.
        assert!((38..=44).contains(&m.width) && (27..=31).contains(&m.height), "{}×{}", m.width, m.height);
        assert!(m.rooms.area() > 100.0, "дома не залиты: {}", m.rooms.area());
        // Между домами — общая стена, наружу — контур; забор — отдельно, низкий.
        let high: Vec<&Wall> = m.walls.iter().filter(|w| !w.low).collect();
        assert_eq!(high.len(), 1, "{:?}", m.walls);
        assert!(high[0].style.as_ref().is_some_and(|s| matches!(&s.paint, Paint::Texture(k) if k.contains("wall_brick"))));
        assert!(m.walls.iter().any(|w| w.low && w.style.as_ref().is_some_and(|s| matches!(&s.paint, Paint::Texture(k) if k.contains("wall_wood")))), "забора нет");
        assert!(!m.room_edges().is_empty());
        // Дверь у отмеченного входа, на стене, а не в воздухе.
        let entrance = Proj { north: 51.5304, west: -0.1250, kx: 111_320.0 * 51.5302f64.to_radians().cos() / CELL_M as f64, ky: 110_574.0 / CELL_M as f64 }.at(51.5302, -0.12465);
        let door = m.portals.iter().find(|p| p.kind.is_door()).expect("дверь");
        let mid = [(door.a[0] + door.b[0]) / 2.0, (door.a[1] + door.b[1]) / 2.0];
        assert!(length(mid, entrance) < 1.0, "дверь {mid:?}, вход {entrance:?}");
        let cut = crate::map::cut_portals(&m.room_edges(), &m.portals);
        assert!(cut.len() > m.room_edges().len(), "дверь не прорезала стену");
        // У лавки входа нет: дверь — к улице на юге, а не к дорожке позади.
        let doors: Vec<[f32; 2]> = m.portals.iter().filter(|p| p.kind.is_door()).map(|p| [(p.a[0] + p.b[0]) / 2.0, (p.a[1] + p.b[1]) / 2.0]).collect();
        assert_eq!(doors.len(), 2, "{doors:?}");
        let shop = doors.iter().find(|d| length(**d, entrance) > 2.0).expect("дверь лавки");
        assert!(shop[1] > 12.0, "дверь лавки смотрит не на улицу: {shop:?}");
    }

    #[test]
    fn streets_rails_trees_and_labels() {
        let (m, _) = sample();
        let tex = |p: &Path| match &m.materials[p.material].paint {
            Paint::Texture(k) => k.clone(),
            Paint::Color(_) => "color".into(),
        };
        assert!(m.paths.iter().any(|p| tex(p).contains("cobblestone") && (p.width - 6.0 / CELL_M).abs() < 0.01), "улицы нет");
        assert!(m.paths.iter().any(|p| tex(p).contains("gravel")), "насыпи нет");
        assert_eq!(m.paths.iter().filter(|p| tex(p) == "color").count(), 2, "рельсов должно быть два");
        // Подземная платформа (около 7,3 клетки от верха) не нарисована; плитка — только дорожка (около 5,1).
        let tiles: Vec<&Path> = m.paths.iter().filter(|p| tex(p).contains("stone_tiles")).collect();
        assert_eq!(tiles.len(), 1, "подземная платформа на карте");
        assert!(tiles[0].points.iter().all(|q| (q[1] - 5.1).abs() < 0.3), "{:?}", tiles[0].points);
        let count = |word: &str| m.props.iter().filter(|q| q.asset.contains(word)).count();
        assert_eq!((count("builtin:tree"), count("street_lamp"), count("bench")), (1, 1, 1), "{:?}", m.props);
        let bench = m.props.iter().find(|q| q.asset.contains("bench")).unwrap();
        assert!(bench.rot.abs() < 1.0 || (bench.rot - 180.0).abs() < 1.0, "скамья вдоль улицы: {}", bench.rot);
        assert!(m.labels.iter().any(|l| l.text == "Crowfall Lane" && l.gm && l.rot.abs() < 1.0));
    }

    /// Красная площадь в малом: брусчатая площадь слева, пешеходная линия через
    /// неё и дальше, лавка справа.
    #[test]
    fn square_keeps_its_paving_and_shop_floor_differs() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<osm version="0.6">
 <bounds minlat="51.5300" minlon="-0.1250" maxlat="51.5304" maxlon="-0.1241"/>
 <node id="1" lat="51.5301" lon="-0.1249"/>
 <node id="2" lat="51.5301" lon="-0.1245"/>
 <node id="3" lat="51.5303" lon="-0.1245"/>
 <node id="4" lat="51.5303" lon="-0.1249"/>
 <way id="10"><nd ref="1"/><nd ref="2"/><nd ref="3"/><nd ref="4"/><nd ref="1"/><tag k="highway" v="pedestrian"/><tag k="area" v="yes"/><tag k="surface" v="sett"/></way>
 <node id="11" lat="51.5302" lon="-0.12485"/>
 <node id="12" lat="51.5302" lon="-0.1247"/>
 <node id="13" lat="51.5302" lon="-0.1242"/>
 <way id="20"><nd ref="11"/><nd ref="12"/><nd ref="13"/><tag k="highway" v="footway"/></way>
 <node id="21" lat="51.53005" lon="-0.1244"/>
 <node id="22" lat="51.53005" lon="-0.1242"/>
 <node id="23" lat="51.53015" lon="-0.1242"/>
 <node id="24" lat="51.53015" lon="-0.1244"/>
 <way id="30"><nd ref="21"/><nd ref="22"/><nd ref="23"/><nd ref="24"/><nd ref="21"/><tag k="building" v="retail"/></way>
</osm>"#;
        let (m, _) = import(xml, Options::default()).unwrap();
        let tex = |mat: usize| match &m.materials[mat].paint {
            Paint::Texture(k) => k.clone(),
            Paint::Color(_) => "color".into(),
        };
        // Площадь — брусчаткой по `surface=sett`, как улица.
        assert!(m.terrain.iter().any(|c| tex(c.material as usize).contains("cobblestone")), "{:?}", m.terrain.iter().map(|c| tex(c.material as usize)).collect::<Vec<_>>());
        // Дорожка — только за площадью (её край — около 22,7 клетки). Других линий нет.
        let walk: Vec<&Path> = m.paths.iter().collect();
        assert!(!walk.is_empty(), "дорожка за площадью пропала");
        assert!(walk.iter().flat_map(|p| p.points.iter()).all(|q| q[0] > 22.0), "дорожка легла по площади: {:?}", walk.iter().map(|p| &p.points).collect::<Vec<_>>());
        // Пол лавки — дерево, не плитка мостовой.
        assert!(m.room_floor.iter().any(|c| tex(c.material as usize).contains("wood_planks")), "{:?}", m.room_floor.iter().map(|c| tex(c.material as usize)).collect::<Vec<_>>());
    }

    /// Город в малом (41×29 клеток): дом в углу справа сверху (больше 6 % рамки) и
    /// улица внизу, а слева — то, что подставит проверка (`{}`).
    fn town(left: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<osm version="0.6">
 <bounds minlat="51.5300" minlon="-0.1250" maxlat="51.5304" maxlon="-0.1241"/>
 <node id="901" lat="51.5304" lon="-0.12430"/>
 <node id="902" lat="51.5304" lon="-0.1241"/>
 <node id="903" lat="51.53026" lon="-0.1241"/>
 <node id="904" lat="51.53026" lon="-0.12430"/>
 <way id="900"><nd ref="901"/><nd ref="902"/><nd ref="903"/><nd ref="904"/><nd ref="901"/><tag k="building" v="yes"/></way>
 <node id="911" lat="51.530055" lon="-0.1250"/>
 <node id="912" lat="51.530055" lon="-0.1241"/>
 <way id="910"><nd ref="911"/><nd ref="912"/><tag k="highway" v="residential"/></way>
 <node id="1" lat="51.5304" lon="-0.1250"/>
 <node id="2" lat="51.5304" lon="-0.124671"/>
 <node id="3" lat="51.53012" lon="-0.124671"/>
 <node id="4" lat="51.53012" lon="-0.1250"/>
 <node id="5" lat="51.5304" lon="-0.124649"/>
 <node id="6" lat="51.5304" lon="-0.12434"/>
 <node id="7" lat="51.53012" lon="-0.12434"/>
 <node id="8" lat="51.53012" lon="-0.124649"/>
 {left}
</osm>"#
        )
    }

    fn floor_tex(m: &Map, x: i32, y: i32) -> String {
        match m.floor_at(x, y) {
            0 => String::new(),
            k => match &m.materials[k as usize - 1].paint {
                Paint::Texture(t) => t.clone(),
                Paint::Color(_) => "color".into(),
            },
        }
    }

    #[test]
    fn osm_colours_read_as_hex_and_names() {
        assert_eq!(colour("#ec8859"), Some([236, 136, 89]));
        assert_eq!(colour("#FFF"), Some([255, 255, 255]));
        assert_eq!(colour(" Firebrick "), Some([178, 34, 34]));
        assert_eq!(colour("grey"), colour("gray"));
        assert_eq!(colour("#12345"), None);
        assert_eq!(colour("кирпичный"), None);
    }

    /// Кремль в малом: стена в 3 клетки шириной (`building=wall`, верх `#ec8859`)
    /// упирается в боевую башню. Стена — кирпич в своём цвете с зубцами, без
    /// дверей; башня — кирпич в цвет стен, стена между ними — тоже в цвет.
    #[test]
    fn kremlin_wall_is_red_brick_with_teeth_and_no_doors() {
        let xml = town(
            r##"<node id="21" lat="51.53029" lon="-0.124956"/>
 <node id="22" lat="51.53029" lon="-0.124429"/>
 <node id="23" lat="51.530248" lon="-0.124429"/>
 <node id="24" lat="51.530248" lon="-0.124956"/>
 <way id="20"><nd ref="21"/><nd ref="22"/><nd ref="23"/><nd ref="24"/><nd ref="21"/><tag k="building" v="wall"/><tag k="building:material" v="brick"/><tag k="roof:colour" v="#ec8859"/></way>
 <node id="31" lat="51.530317" lon="-0.124429"/>
 <node id="32" lat="51.530317" lon="-0.124341"/>
 <node id="33" lat="51.530221" lon="-0.124341"/>
 <node id="34" lat="51.530221" lon="-0.124429"/>
 <way id="30"><nd ref="31"/><nd ref="32"/><nd ref="33"/><nd ref="34"/><nd ref="31"/><tag k="building" v="yes"/><tag k="tower:type" v="defensive"/><tag k="building:colour" v="#ec8859"/><tag k="building:material" v="brick"/></way>"##,
        );
        let (m, _) = import(&xml, Options::default()).unwrap();
        let room = |x: i32, y: i32| match m.room_at(x, y) {
            0 => String::new(),
            k => match &m.materials[k as usize - 1].paint {
                Paint::Texture(t) => t.clone(),
                Paint::Color(_) => "color".into(),
            },
        };
        for (what, x) in [("стена", 14), ("башня", 28)] {
            assert!(room(x, 9).starts_with("tint:ec8859:") && room(x, 9).contains("brick"), "{what}: {}", room(x, 9));
        }
        let teeth: Vec<&Wall> = m.walls.iter().filter(|w| w.low && matches!(&w.style, Some(s) if matches!(&s.paint, Paint::Texture(k) if k.starts_with("tint:a96140:")))).collect();
        // Стена 23 клетки: по 10 зубцов с каждой длинной стороны; у башни — ни одного.
        assert!(teeth.len() >= 18, "зубцов {}", teeth.len());
        assert!(teeth.iter().flat_map(|w| &w.points).all(|p| p[0] < 25.9 && (7.9..11.4).contains(&p[1])), "зубец не на стене: {:?}", teeth.iter().map(|w| &w.points).collect::<Vec<_>>());
        let on_wall = |p: [f32; 2]| (2.0..25.5).contains(&p[0]) && (7.5..11.5).contains(&p[1]);
        assert!(m.portals.iter().all(|d| !on_wall(d.a) && !on_wall(d.b)), "дверь в стене: {:?}", m.portals.iter().map(|d| (d.a, d.b)).collect::<Vec<_>>());
        assert!(m.walls.iter().any(|w| matches!(&w.style, Some(s) if matches!(&s.paint, Paint::Texture(k) if k.starts_with("tint:ec8859:")))), "стена между стеной и башней не в цвет");
    }

    /// Кремлёвский парк в малом: парк на всю левую часть, на снимке трава лишь
    /// в левой трети — справа мощёная площадка. Трава парка — только по снимку.
    #[test]
    fn park_is_grass_only_where_the_photo_is_green() {
        let xml = town(r#"<way id="10"><nd ref="1"/><nd ref="6"/><nd ref="7"/><nd ref="4"/><nd ref="1"/><tag k="leisure" v="park"/></way>"#);
        let (bare, _) = import(&xml, Options::default()).unwrap();
        assert!(floor_tex(&bare, 25, 10).contains("grass"), "без снимка парк — трава: {}", floor_tex(&bare, 25, 10));
        let cover = Cover::grid(82, 58, |x, _| if x < 30 { Class::Grass } else { Class::Earth });
        let (m, _) = import_cover(&xml, &[], Some(&cover), Options::default()).unwrap();
        assert!(floor_tex(&m, 5, 10).contains("grass"), "газон парка: {}", floor_tex(&m, 5, 10));
        assert!(!floor_tex(&m, 25, 10).contains("grass"), "мощёное в парке стало газоном: {}", floor_tex(&m, 25, 10));
    }

    /// Васильевский спуск: в OSM грунт, а на снимке — как брусчатая площадь рядом.
    /// Бурое на снимке под той же площадью — и правда грунт.
    #[test]
    fn dirt_square_that_looks_like_the_paving_next_to_it_is_paved() {
        let xml = town(
            r#"<way id="10"><nd ref="1"/><nd ref="2"/><nd ref="3"/><nd ref="4"/><nd ref="1"/><tag k="highway" v="pedestrian"/><tag k="area" v="yes"/><tag k="surface" v="sett"/></way>
 <way id="20"><nd ref="5"/><nd ref="6"/><nd ref="7"/><nd ref="8"/><nd ref="5"/><tag k="highway" v="pedestrian"/><tag k="area" v="yes"/><tag k="surface" v="dirt"/><tag k="name" v="Спуск"/></way>"#,
        );
        let photo = |dirt: image::Rgb<u8>| image::RgbImage::from_fn(82, 58, move |x, _| if x < 32 { image::Rgb([96, 98, 100]) } else { dirt });
        let grey = Cover::grid(82, 58, |_, _| Class::Earth).with_photo(photo(image::Rgb([118, 120, 117])));
        let (m, r) = import_cover(&xml, &[], Some(&grey), Options::default()).unwrap();
        assert!(floor_tex(&m, 22, 10).contains("cobblestone"), "серая «грунтовая» площадь: {}", floor_tex(&m, 22, 10));
        assert!(r.warnings.iter().any(|w| w.contains("Спуск") && w.contains("dirt")), "{:?}", r.warnings);
        let brown = Cover::grid(82, 58, |_, _| Class::Earth).with_photo(photo(image::Rgb([150, 112, 72])));
        let (m, _) = import_cover(&xml, &[], Some(&brown), Options::default()).unwrap();
        assert!(floor_tex(&m, 22, 10).contains("dirt"), "бурая площадь: {}", floor_tex(&m, 22, 10));
        // Без снимка — как в OSM.
        let (m, _) = import(&xml, Options::default()).unwrap();
        assert!(floor_tex(&m, 22, 10).contains("dirt"), "без снимка: {}", floor_tex(&m, 22, 10));
    }

    /// Город: крона снимка в лесу — дерево, на брусчатой площади — нет (там на
    /// снимке тёмное — и тени домов). В городском лесу без крон не росло ни дерева.
    #[test]
    fn town_crowns_grow_in_the_wood_but_not_on_the_square() {
        let xml = town(
            r#"<way id="10"><nd ref="1"/><nd ref="2"/><nd ref="3"/><nd ref="4"/><nd ref="1"/><tag k="natural" v="wood"/></way>
 <way id="20"><nd ref="5"/><nd ref="6"/><nd ref="7"/><nd ref="8"/><nd ref="5"/><tag k="highway" v="pedestrian"/><tag k="area" v="yes"/><tag k="surface" v="sett"/></way>"#,
        );
        let (bare, _) = import(&xml, Options::default()).unwrap();
        let (w, h) = (bare.width as f32, bare.height as f32);
        let (wood, square) = ([7.0, 9.0], [23.0, 9.0]);
        let json = format!(
            r#"{{"crowns": [{{"u": {}, "v": {}, "r": 5.0, "kind": "broad", "src": "none", "queue": 0.5}}, {{"u": {}, "v": {}, "r": 5.0, "kind": "broad", "src": "none", "queue": 0.5}}]}}"#,
            wood[0] / w,
            wood[1] / h,
            square[0] / w,
            square[1] / h
        );
        let cover = Cover::grid(82, 58, |_, _| Class::Grass).with_crowns(&json).unwrap();
        let (m, _) = import_cover(&xml, &[], Some(&cover), Options::default()).unwrap();
        let crowns = crowns_of(&m);
        assert!(crowns.iter().any(|c| length(c.0, wood) < 1.0), "в лесу нет дерева: {crowns:?}");
        assert!(crowns.iter().all(|c| length(c.0, square) > 3.0), "дерево на площади: {crowns:?}");
    }

    /// Природа: лес слева, озеро справа, гравийная тропа через лес, каменная
    /// ограда с калиткой, камень. Домов нет.
    const RURAL: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<osm version="0.6">
 <bounds minlat="51.5300" minlon="-0.1250" maxlat="51.5304" maxlon="-0.1241"/>
 <node id="1" lat="51.5300" lon="-0.1250"/>
 <node id="2" lat="51.5300" lon="-0.1246"/>
 <node id="3" lat="51.5304" lon="-0.1246"/>
 <node id="4" lat="51.5304" lon="-0.1250"/>
 <way id="10"><nd ref="1"/><nd ref="2"/><nd ref="3"/><nd ref="4"/><nd ref="1"/><tag k="natural" v="wood"/><tag k="leaf_type" v="broadleaved"/></way>
 <node id="11" lat="51.53005" lon="-0.12445"/>
 <node id="12" lat="51.53005" lon="-0.1242"/>
 <node id="13" lat="51.53025" lon="-0.1242"/>
 <node id="14" lat="51.53025" lon="-0.12445"/>
 <way id="20"><nd ref="11"/><nd ref="12"/><nd ref="13"/><nd ref="14"/><nd ref="11"/><tag k="natural" v="water"/><tag k="name" v="Tarn"/></way>
 <node id="21" lat="51.5302" lon="-0.1251"/>
 <node id="22" lat="51.5302" lon="-0.1245"/>
 <way id="30"><nd ref="21"/><nd ref="22"/><tag k="highway" v="path"/><tag k="surface" v="gravel"/></way>
 <node id="31" lat="51.53035" lon="-0.1245"/>
 <node id="32" lat="51.53035" lon="-0.1243"><tag k="barrier" v="gate"/></node>
 <node id="33" lat="51.53035" lon="-0.1241"/>
 <way id="40"><nd ref="31"/><nd ref="32"/><nd ref="33"/><tag k="barrier" v="wall"/></way>
 <node id="50" lat="51.53032" lon="-0.12425"><tag k="natural" v="stone"/></node>
</osm>"#;

    #[test]
    fn countryside_grass_forest_lake_path_wall() {
        let (m, r) = import(RURAL, Options::default()).unwrap();
        assert!(r.warnings.is_empty(), "{:?}", r.warnings);
        assert_eq!(m.ambience, "forest-summer");
        let key = |x: i32, y: i32| match m.floor_at(x, y) {
            0 => String::new(),
            k => match &m.materials[k as usize - 1].paint {
                Paint::Texture(t) => t.clone(),
                Paint::Color(_) => "color".into(),
            },
        };
        // Под всем трава (здесь нет ни леса, ни воды, ни тропы).
        assert!(key(38, 3).contains("grass"), "{}", key(38, 3));
        // Озеро: середина глубокая, у края (около x = 25, край гуляет) мелко.
        assert!(key(30, 18).contains("water_deep"), "{}", key(30, 18));
        assert!((23..=28).any(|x| key(x, 18).contains("water_shallow")), "{:?}", (23..=28).map(|x| key(x, 18)).collect::<Vec<_>>());
        // Деревья в лесу (опушка около 18,2 клетки, гуляет на 0,7) и не на тропе (y ≈ 14,5).
        // На ничьей земле справа — разве что одинокие, и не в озере (25–36 × 11–25).
        let trees: Vec<&Prop> = m.props.iter().filter(|q| q.asset.contains("builtin:tree")).collect();
        let (wood, lone): (Vec<&Prop>, Vec<&Prop>) = trees.iter().partition(|q| q.pos[0] < 19.0);
        assert!(wood.len() >= 15, "деревьев {}", wood.len());
        assert!(wood.iter().all(|q| (q.pos[1] - 14.5).abs() > 1.0), "{:?}", wood.iter().map(|q| q.pos).collect::<Vec<_>>());
        assert!(lone.len() <= 4 && lone.iter().all(|q| !((24.5..37.0).contains(&q.pos[0]) && (10.5..26.0).contains(&q.pos[1]))), "{:?}", lone.iter().map(|q| q.pos).collect::<Vec<_>>());
        assert!(m.terrain.iter().any(|c| matches!(&m.materials[c.material as usize].paint, Paint::Texture(t) if t.contains("forest_floor"))), "подстилки нет");
        // Тропа гравийная, край мягкий.
        let path = m.paths.iter().find(|p| matches!(&m.materials[p.material].paint, Paint::Texture(t) if t.contains("gravel"))).expect("тропа");
        assert!(path.soft >= 0.25 && path.soft < 0.45, "гравий ровнее земляной тропы: {}", path.soft);
        // Ограда — сухая кладка, низкая, разорвана у калитки.
        assert_eq!(m.walls.len(), 2, "{:?}", m.walls);
        assert!(m.walls.iter().all(|w| w.low && w.style.as_ref().is_some_and(|s| matches!(&s.paint, Paint::Texture(t) if t.contains("fieldstone")))));
        let gap = length(m.walls[0].points[1], m.walls[1].points[0]);
        assert!((gap - 2.0).abs() < 0.05, "проход {gap}");
        assert!(m.props.iter().any(|q| q.asset.contains("boulder")), "камня нет");
        assert!(m.labels.iter().any(|l| l.text == "Tarn"));
    }

    /// Кладбище слева, речной пляж без покрытия справа; моря в выгрузке нет.
    const CHURCHYARD: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<osm version="0.6">
 <bounds minlat="51.5300" minlon="-0.1250" maxlat="51.5304" maxlon="-0.1241"/>
 <node id="1" lat="51.5301" lon="-0.1249"/>
 <node id="2" lat="51.5301" lon="-0.1245"/>
 <node id="3" lat="51.5303" lon="-0.1245"/>
 <node id="4" lat="51.5303" lon="-0.1249"/>
 <way id="10"><nd ref="1"/><nd ref="2"/><nd ref="3"/><nd ref="4"/><nd ref="1"/><tag k="amenity" v="grave_yard"/></way>
 <node id="11" lat="51.5301" lon="-0.1244"/>
 <node id="12" lat="51.5301" lon="-0.1242"/>
 <node id="13" lat="51.5303" lon="-0.1242"/>
 <node id="14" lat="51.5303" lon="-0.1244"/>
 <way id="20"><nd ref="11"/><nd ref="12"/><nd ref="13"/><nd ref="14"/><nd ref="11"/><tag k="natural" v="beach"/></way>
</osm>"#;

    #[test]
    fn churchyard_has_graves_in_rows_and_river_beach_is_shingle() {
        let (m, _) = import(CHURCHYARD, Options::default()).unwrap();
        // Кладбище — около 4,5–22,7 × 7,3–21,8 клетки (край гуляет на 0,7).
        let yard = |p: [f32; 2]| (3.5..23.7).contains(&p[0]) && (6.3..22.8).contains(&p[1]);
        // Без надгробий в паках могила — камень у изголовья.
        let stones: Vec<[f32; 2]> = m.props.iter().filter(|q| q.asset.contains("boulder")).map(|q| q.pos).collect();
        assert!(stones.len() >= 8 && stones.iter().all(|&p| yard(p)), "{stones:?}");
        // На кладбище не растёт случайное: оно не ничья земля.
        assert!(m.props.iter().filter(|q| q.asset.contains("tree") || q.asset.contains("bush")).all(|q| !yard(q.pos)));
        let floor = match m.floor_at(31, 14) {
            0 => String::new(),
            k => match &m.materials[k as usize - 1].paint {
                Paint::Texture(t) => t.clone(),
                Paint::Color(_) => "color".into(),
            },
        };
        assert!(floor.contains("gravel"), "речной пляж — галька: {floor}");
    }

    /// Берег моря из двух путей с запада на восток (суша слева — на севере) и остров
    /// в море (кольцо против часовой — суша внутри). Моря площадью нет.
    const COAST: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<osm version="0.6">
 <bounds minlat="51.5300" minlon="-0.1250" maxlat="51.5304" maxlon="-0.1241"/>
 <node id="61" lat="51.53025" lon="-0.1253"/>
 <node id="62" lat="51.53025" lon="-0.12455"/>
 <node id="63" lat="51.53022" lon="-0.1238"/>
 <way id="60"><nd ref="61"/><nd ref="62"/><tag k="natural" v="coastline"/></way>
 <way id="64"><nd ref="62"/><nd ref="63"/><tag k="natural" v="coastline"/></way>
 <node id="71" lat="51.53005" lon="-0.1247"/>
 <node id="72" lat="51.53005" lon="-0.1245"/>
 <node id="73" lat="51.53013" lon="-0.1245"/>
 <node id="74" lat="51.53013" lon="-0.1247"/>
 <way id="70"><nd ref="71"/><nd ref="72"/><nd ref="73"/><nd ref="74"/><nd ref="71"/><tag k="natural" v="coastline"/></way>
 <node id="81" lat="51.53027" lon="-0.1243"/>
 <node id="82" lat="51.53016" lon="-0.1243"/>
 <way id="80"><nd ref="81"/><nd ref="82"/><tag k="man_made" v="pier"/></way>
</osm>"#;

    #[test]
    fn sea_from_coastline_with_island_and_cut_line() {
        let floor = |m: &Map, x: i32, y: i32| match m.floor_at(x, y) {
            0 => String::new(),
            k => match &m.materials[k as usize - 1].paint {
                Paint::Texture(t) => t.clone(),
                Paint::Color(_) => "color".into(),
            },
        };
        let (m, _) = import(COAST, Options::default()).unwrap();
        // Берег около y = 10,9; остров — 13,6–22,7 × 19,6–25,4 клетки.
        for (x, y) in [(3, 3), (20, 5), (38, 4), (18, 22)] {
            assert!(floor(&m, x, y).contains("grass"), "суша ({x}, {y}): {}", floor(&m, x, y));
        }
        for (x, y) in [(3, 20), (30, 15), (38, 27), (18, 15), (8, 27)] {
            assert!(floor(&m, x, y).contains("water"), "море ({x}, {y}): {}", floor(&m, x, y));
        }
        // У берега мелко, глубина набирается плавно — на 15 клетках уже есть, но не вся.
        assert!(floor(&m, 3, 13).contains("water_shallow"), "у берега мелко: {}", floor(&m, 3, 13));
        let deep = m.materials.iter().position(|t| matches!(&t.paint, Paint::Texture(t) if t.contains("water_deep"))).expect("глубины нет");
        let depth = |x: f32, y: f32| m.terrain.iter().filter(|l| l.material as usize == deep).map(|l| l.mask.get((x * mask::RES as f32) as isize, (y * mask::RES as f32) as isize)).max().unwrap_or(0);
        assert!(depth(3.0, 13.0) < depth(3.0, 20.0) && depth(3.0, 20.0) < depth(3.0, 27.0) && depth(3.0, 27.0) < 255, "{} {} {}", depth(3.0, 13.0), depth(3.0, 20.0), depth(3.0, 27.0));
        // Под полупрозрачной водой моря — песчаное дно; причал — доски вдоль.
        assert!(m.materials.iter().any(|t| matches!(&t.paint, Paint::Texture(t) if t.contains("sand"))), "дна нет");
        assert!(m.paths.iter().any(|p| p.along && matches!(&m.materials[p.material].paint, Paint::Texture(t) if t.contains("plank"))), "причала нет");
        // В море ничего не растёт.
        let wet = |p: [f32; 2]| p[1] > 12.5 && !((13.0..23.5).contains(&p[0]) && (19.0..26.0).contains(&p[1]));
        assert!(m.props.iter().all(|q| !wet(q.pos)), "{:?}", m.props.iter().filter(|q| wet(q.pos)).map(|q| (&q.asset, q.pos)).collect::<Vec<_>>());
        // Выгрузка оборвала берег посреди карты — линия продлевается по ходу до края.
        let cut = COAST.replace(r#"<way id="64"><nd ref="62"/><nd ref="63"/><tag k="natural" v="coastline"/></way>"#, "");
        let (m, _) = import(&cut, Options::default()).unwrap();
        assert!(floor(&m, 35, 20).contains("water"), "{}", floor(&m, 35, 20));
        assert!(floor(&m, 35, 4).contains("grass"), "{}", floor(&m, 35, 4));
    }

    /// Обрыв поперёк карты с запада на восток (y ≈ 13 клеток): верх по OSM — на севере.
    const CLIFF: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<osm version="0.6">
 <bounds minlat="51.5300" minlon="-0.1250" maxlat="51.5304" maxlon="-0.1241"/>
 <node id="1" lat="51.53022" lon="-0.1253"/>
 <node id="2" lat="51.53022" lon="-0.12455"/>
 <node id="3" lat="51.53022" lon="-0.1238"/>
 <way id="10"><nd ref="1"/><nd ref="2"/><nd ref="3"/><tag k="natural" v="cliff"/></way>
</osm>"#;

    #[test]
    fn cliff_raises_its_left_side_into_relief_cliff() {
        let (m, _) = import(CLIFF, Options::default()).unwrap();
        let r = m.relief.as_ref().expect("рельефа нет");
        assert_eq!((r.w, r.h), (m.rooms.w, m.rooms.h));
        // Без `height` — 3 яруса: север (меньший y) выше юга. Опускается юг —
        // основная земля остаётся на верхнем ярусе и не темнеет.
        assert!(r.at([20.0, 10.0]).abs() < 0.1, "север {}", r.at([20.0, 10.0]));
        assert!((r.at([20.0, 15.0]) + 3.0).abs() < 0.1, "юг {}", r.at([20.0, 15.0]));
        // Дальше от обрыва земля поднимается обратно — полого, без колец гребней.
        assert!(r.at([20.0, 28.0]) > r.at([20.0, 17.0]) + 1.0, "дальше от обрыва земля поднимается обратно: {} {}", r.at([20.0, 17.0]), r.at([20.0, 28.0]));
        assert!(r.natural, "рельеф настоящей местности");
        assert!(crate::relief::edges(r).iter().all(|e| e.kind == crate::relief::EdgeKind::Cliff), "пологий подъём — без гребней");
        let cliffs: Vec<crate::relief::Edge> = crate::relief::edges(r).into_iter().filter(|e| e.kind == crate::relief::EdgeKind::Cliff).collect();
        assert!(!cliffs.is_empty(), "обрыва нет");
        // Обрыв один — по линии; верх смотрит на север. Дальний край плато спадает
        // гребнями, а не вторым обрывом.
        for e in &cliffs {
            assert!(e.points.iter().all(|p| (p[1] - 13.0).abs() < 2.0), "обрыв не на линии: {:?}", &e.points[..3.min(e.points.len())]);
            let k = e.points.len() / 2;
            assert!(e.up(k)[1] < -0.5, "верх не на севере: {:?}", e.up(k));
        }
        // Тот же обрыв, проведённый на запад, поднимает юг; `height=6` — 4 яруса.
        let west = CLIFF.replace(r#"<nd ref="1"/><nd ref="2"/><nd ref="3"/>"#, r#"<nd ref="3"/><nd ref="2"/><nd ref="1"/>"#).replace(r#"v="cliff"/>"#, r#"v="cliff"/><tag k="height" v="6"/>"#);
        let (m, _) = import(&west, Options::default()).unwrap();
        let r = m.relief.as_ref().unwrap();
        assert!(r.at([20.0, 15.0]).abs() < 0.1 && (r.at([20.0, 11.0]) + 4.0).abs() < 0.1, "{} {}", r.at([20.0, 15.0]), r.at([20.0, 11.0]));
        // Обрыв — рельеф, а не тёмная стена.
        assert!(m.walls.is_empty(), "{:?}", m.walls);
    }

    #[test]
    fn cover_puts_trees_where_the_photo_has_crowns() {
        // Пустой участок за городом; на снимке кроны в западной половине, трава — в восточной.
        const EMPTY: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<osm version="0.6">
 <bounds minlat="51.5300" minlon="-0.1250" maxlat="51.5304" maxlon="-0.1241"/>
</osm>"#;
        let cover = Cover::grid(80, 80, |x, _| if x < 40 { Class::Trees } else { Class::Grass });
        let (m, _) = import_cover(EMPTY, &[], Some(&cover), Options::default()).unwrap();
        let half = m.width as f32 / 2.0;
        let trees: Vec<[f32; 2]> = m.props.iter().filter(|p| p.asset.contains("tree") || p.asset.contains("pine")).map(|p| p.pos).collect();
        let west = trees.iter().filter(|p| p[0] < half).count();
        assert!(west >= 6, "на западе деревьев {west}: {trees:?}");
        assert!(trees.iter().all(|p| p[0] < half + 1.5), "дерево на траве: {:?}", trees.iter().filter(|p| p[0] >= half + 1.5).collect::<Vec<_>>());
        // Сплошной полог — кроны по 10–17 м, смыкаются, но не наползают.
        let crowns = crowns_of(&m);
        for (i, a) in crowns.iter().enumerate() {
            assert!(a.1 >= CROWN_MIN && a.1 <= CROWN_CANOPY.1 + 1e-3, "крона полога {a:?}");
            assert!(crowns[i + 1..].iter().all(|b| length(a.0, b.0) >= CROWN_OVERLAP * (a.1 + b.1) - 1e-3), "кроны слиплись у {a:?}");
        }
        assert!(crowns.iter().any(|c| c.1 >= CROWN_CANOPY.0), "в пологе одна мелочь: {crowns:?}");
    }

    /// Кроны деревьев карты: центр и видимый радиус в клетках.
    fn crowns_of(m: &Map) -> Vec<([f32; 2], f32)> {
        m.props.iter().filter(|p| p.asset.contains("tree") || p.asset.contains("pine")).map(|p| (p.pos, p.scale * 0.85 * p.size[0].max(p.size[1]) / 2.0)).collect()
    }

    #[test]
    fn base_floor_follows_the_biome_of_the_photo() {
        let mix = |parts: &'static [(Class, usize)]| {
            Cover::grid(100, 1, move |x, _| {
                let mut x = x;
                for (c, n) in parts {
                    if x < *n {
                        return *c;
                    }
                    x -= n;
                }
                Class::Grass
            })
        };
        // Англия: трава и кроны, сухое пятнами — луг; вода в счёт не идёт.
        assert_eq!(mix(&[(Class::Water, 44), (Class::Grass, 30), (Class::Trees, 14), (Class::Rock, 11), (Class::Dry, 1)]).base_floor(), "meadow");
        // Табернас: земля 65 %, камень 26 %.
        assert_eq!(mix(&[(Class::Earth, 65), (Class::Rock, 26), (Class::Trees, 6), (Class::Grass, 3)]).base_floor(), "earth_pale");
        // Трухильо: сухое 35 %, бурое 15 %, земля 28 %, трава 13 %.
        assert_eq!(mix(&[(Class::Dry, 35), (Class::Brown, 15), (Class::Earth, 28), (Class::Grass, 13), (Class::Trees, 9)]).base_floor(), "straw_grass");
        // Мер-де-Глас: серый лёд 61 %, морена 32 % — лёд.
        assert_eq!(mix(&[(Class::Ice, 61), (Class::Rock, 32), (Class::Snow, 7)]).base_floor(), "ice_glacier");
        // Мёнхсйох: белый снег и фирн 45 %, скала 50 % — снег, хоть в OSM и ледник.
        assert_eq!(mix(&[(Class::Snow, 45), (Class::Rock, 50), (Class::Ice, 5)]).base_floor(), "snow");
    }

    #[test]
    fn desert_photo_lays_bare_earth_with_green_where_it_grows() {
        const EMPTY: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<osm version="0.6">
 <bounds minlat="51.5300" minlon="-0.1250" maxlat="51.5304" maxlon="-0.1241"/>
</osm>"#;
        // Голая земля, на западе — зелёный луг в треть рамки.
        let cover = Cover::grid(60, 60, |x, _| if x < 20 { Class::Grass } else { Class::Earth });
        let (m, _) = import_cover(EMPTY, &[], Some(&cover), Options::default()).unwrap();
        let floor = |x: i32, y: i32| match m.floor_at(x, y) {
            0 => String::new(),
            k => match &m.materials[k as usize - 1].paint {
                Paint::Texture(t) => t.clone(),
                Paint::Color(_) => "color".into(),
            },
        };
        let (w, h) = (m.width as i32, m.height as i32);
        let (east, west) = (floor(w * 5 / 6, h / 2), floor(w / 8, h / 2));
        assert!(east.contains("dirt"), "в пустыне не земля: {east}");
        assert_ne!(east, west, "зелёное пятно не легло");
    }

    #[test]
    fn groves_on_request_grow_on_land_and_not_in_water() {
        const EMPTY: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<osm version="0.6">
 <bounds minlat="51.5300" minlon="-0.1250" maxlat="51.5304" maxlon="-0.1241"/>
</osm>"#;
        // Луг на западе, вода по снимку на востоке.
        let cover = Cover::grid(60, 60, |x, _| if x < 30 { Class::Grass } else { Class::Water });
        let plants = |trees: f32| {
            let (m, _) = import_cover(EMPTY, &[], Some(&cover), Options { trees, ..Default::default() }).unwrap();
            let east = m.props.iter().filter(|p| p.pos[0] > m.width as f32 * 0.6).count();
            (m.props.len(), east)
        };
        let (plain, _) = plants(0.0);
        let (groves, wet) = plants(1.0);
        assert!(groves >= plain + 10, "рощ не прибавилось: {plain} → {groves}");
        assert_eq!(wet, 0, "деревья в воде");
    }

    #[test]
    fn glacier_is_ice() {
        let t: Tags = [("natural".to_string(), "glacier".to_string())].into_iter().collect();
        assert_eq!(ground(&t, false).map(|g| g.0), Some("ice_glacier"));
    }

    #[test]
    fn crown_species_comes_from_the_forest_map_then_the_osm_tag() {
        let tags = |kv: &[(&str, &str)]| kv.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<Tags>();
        let needle = growth(&tags(&[("natural", "wood"), ("leaf_type", "needleleaved")])).unwrap();
        let broad = growth(&tags(&[("natural", "wood"), ("leaf_type", "broadleaved")])).unwrap();
        let mixed = growth(&tags(&[("natural", "wood"), ("leaf_type", "mixed")])).unwrap();
        // Куст — по размеру, что бы ни сказал тег леса; малая крона, которую
        // анализ назвал хвойной (ельник по Copernicus), — молодая ель.
        assert_eq!(crown_slot(1.0, None, Some(&needle), 0.0, false), "bush_summer");
        assert_eq!(crown_slot(1.0, Some(true), Some(&needle), 0.0, false), "fir");
        // Карта лесов главнее тега: лиственная крона в «хвойном» лесу OSM.
        assert_eq!(crown_slot(3.0, Some(false), Some(&needle), 0.0, false), "tree_green");
        assert_eq!(crown_slot(3.0, Some(true), None, 0.9, false), "pine");
        assert_eq!(crown_slot(1.8, Some(true), None, 0.9, false), "fir");
        // Без карты — тег леса; смешанный — по очереди.
        assert_eq!(crown_slot(3.0, None, Some(&needle), 0.99, false), "pine");
        assert_eq!(crown_slot(3.0, None, Some(&broad), 0.0, false), "tree_green");
        assert_eq!(crown_slot(3.0, None, Some(&mixed), 0.1, false), "pine");
        assert_eq!(crown_slot(3.0, None, Some(&mixed), 0.9, false), "tree_green");
        // За лесом хвойных наугад нет.
        assert_eq!(crown_slot(3.0, None, None, 0.0, false), "tree_green");
        // Сухой край: каменный дуб и сухой куст; хвойные — как везде.
        assert_eq!(crown_slot(3.0, None, None, 0.0, true), "encina");
        assert_eq!(crown_slot(1.0, None, None, 0.0, true), "matorral");
        assert_eq!(crown_slot(3.0, Some(true), None, 0.0, true), "pine");
    }

    #[test]
    fn crowns_file_puts_each_tree_where_the_analysis_found_it() {
        const EMPTY: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<osm version="0.6">
 <bounds minlat="51.5300" minlon="-0.1250" maxlat="51.5304" maxlon="-0.1241"/>
</osm>"#;
        // Подсказка — сплошная трава: деревья только из файла.
        let json = r#"{"crowns": [{"u": 0.5, "v": 0.5, "r": 6.0, "kind": "broad", "src": "nfi:Broadleaved", "queue": 0.3},
                                  {"u": 0.2, "v": 0.3, "r": 1.7, "kind": "bush", "src": "none", "queue": 0.1}]}"#;
        let cover = Cover::grid(40, 40, |_, _| Class::Grass).with_crowns(json).unwrap();
        let (m, _) = import_cover(EMPTY, &[], Some(&cover), Options::default()).unwrap();
        let crowns = crowns_of(&m);
        assert_eq!(crowns.len(), 1, "деревья: {crowns:?}");
        let (q, r) = crowns[0];
        assert!((q[0] - m.width as f32 / 2.0).abs() < 1.0 && (q[1] - m.height as f32 / 2.0).abs() < 1.0, "дерево не в середине: {q:?}");
        assert!((r - 6.0 / CELL_M).abs() < 0.6, "крона {r} клеток вместо {}", 6.0 / CELL_M);
        assert!(m.props.iter().any(|p| p.asset.contains("bush") && length(p.pos, [0.2 * m.width as f32, 0.3 * m.height as f32]) < 1.5), "куста нет");
    }

    #[test]
    fn one_crown_on_the_photo_is_one_tree_of_its_size() {
        // На лугу два дерева: большое круглое в 15 м и рядом впритык два средних
        // по 9 м. Дротики с ровным шагом ставили на большую крону 3–5 мелких.
        const EMPTY: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<osm version="0.6">
 <bounds minlat="51.5300" minlon="-0.1250" maxlat="51.5304" maxlon="-0.1241"/>
</osm>"#;
        let (bare, _) = import(EMPTY, Options::default()).unwrap();
        let (w, h) = (bare.width as f32, bare.height as f32);
        let disks = [([12.0, 14.0], 5.0), ([27.0, 14.0], 3.0), ([32.5, 14.0], 3.0)];
        let cover = Cover::grid(240, 240, |x, y| {
            let p = [(x as f32 + 0.5) / 240.0 * w, (y as f32 + 0.5) / 240.0 * h];
            if disks.iter().any(|(c, r)| length(p, *c) < *r) { Class::Trees } else { Class::Grass }
        });
        let (m, _) = import_cover(EMPTY, &[], Some(&cover), Options::default()).unwrap();
        let crowns = crowns_of(&m);
        assert_eq!(crowns.len(), 3, "по дереву на крону: {crowns:?}");
        for (c, r) in disks {
            let got = crowns.iter().find(|k| length(k.0, c) < 1.2).unwrap_or_else(|| panic!("нет дерева у {c:?}: {crowns:?}"));
            assert!(got.1 > r * 0.9 && got.1 < r * 1.35, "крона у {c:?} радиусом {} вместо {r}", got.1);
        }
    }

    #[test]
    fn short_cliff_ends_without_a_wall_across_the_lowland() {
        // Обрыв посреди карты: за его концами низина поднимается полого, и торцевой
        // стенки поперёк неё нет — только сама линия.
        let short = CLIFF.replace(r#"lon="-0.1253""#, r#"lon="-0.12485""#).replace(r#"lon="-0.1238""#, r#"lon="-0.12425""#);
        let (m, _) = import(&short, Options::default()).unwrap();
        let r = m.relief.as_ref().expect("рельефа нет");
        for e in crate::relief::edges(r).iter().filter(|e| e.kind == crate::relief::EdgeKind::Cliff) {
            assert!(e.points.iter().all(|p| (p[1] - 13.0).abs() < 2.5), "стенка ушла с линии: {:?}", e.points.iter().find(|p| (p[1] - 13.0).abs() >= 2.5));
        }
    }

    #[test]
    fn footpath_vanishes_on_the_beach() {
        // Пляж полосой посреди карты; береговая тропа по гальке идёт через всю карту.
        const BEACH: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<osm version="0.6">
 <bounds minlat="51.5300" minlon="-0.1250" maxlat="51.5304" maxlon="-0.1241"/>
 <node id="1" lat="51.53025" lon="-0.1247"/>
 <node id="2" lat="51.53025" lon="-0.1244"/>
 <node id="3" lat="51.53015" lon="-0.1244"/>
 <node id="4" lat="51.53015" lon="-0.1247"/>
 <way id="10"><nd ref="1"/><nd ref="2"/><nd ref="3"/><nd ref="4"/><nd ref="1"/><tag k="natural" v="beach"/><tag k="surface" v="gravel"/></way>
 <node id="5" lat="51.53020" lon="-0.12498"/>
 <node id="6" lat="51.53020" lon="-0.12412"/>
 <way id="11"><nd ref="5"/><nd ref="6"/><tag k="highway" v="path"/><tag k="surface" v="pebblestone"/></way>
</osm>"#;
        let (m, _) = import(BEACH, Options::default()).unwrap();
        let path: Vec<[f32; 2]> = m.paths.iter().flat_map(|p| p.points.clone()).collect();
        assert!(!path.is_empty(), "вне пляжа тропа есть");
        // Пляж — x 13,7..27,3 клетки; середина пляжа без тропы.
        assert!(path.iter().all(|p| !(15.5..25.5).contains(&p[0])), "тропа по пляжу: {:?}", path.iter().find(|p| (15.5..25.5).contains(&p[0])));
        // Мощёная дорожка по набережной — видна.
        let (m, _) = import(&BEACH.replace("pebblestone", "paving_stones"), Options::default()).unwrap();
        assert!(m.paths.iter().flat_map(|p| p.points.iter()).any(|p| (15.5..25.5).contains(&p[0])), "мощёная пропала");
    }

    /// Лес-мультиполигон из двух путей; в выгрузке только первый — второй за краем.
    const PART_WOOD: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<osm version="0.6">
 <bounds minlat="51.5300" minlon="-0.1250" maxlat="51.5304" maxlon="-0.1241"/>
 <node id="1" lat="51.53005" lon="-0.1249"/>
 <node id="2" lat="51.53005" lon="-0.1243"/>
 <node id="3" lat="51.53035" lon="-0.1243"/>
 <way id="901"><nd ref="1"/><nd ref="2"/><nd ref="3"/></way>
 <relation id="900"><member type="way" ref="901" role="outer"/><member type="way" ref="902" role="outer"/><tag k="type" v="multipolygon"/><tag k="natural" v="wood"/><tag k="name" v="Half Wood"/></relation>
</osm>"#;

    const PART_WOOD_REST: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<osm version="0.6">
 <node id="1" lat="51.53005" lon="-0.1249"/>
 <node id="3" lat="51.53035" lon="-0.1243"/>
 <node id="4" lat="51.53035" lon="-0.1249"/>
 <way id="902"><nd ref="3"/><nd ref="4"/><nd ref="1"/></way>
 <relation id="900"><member type="way" ref="901" role="outer"/><member type="way" ref="902" role="outer"/><tag k="type" v="multipolygon"/><tag k="natural" v="wood"/><tag k="name" v="Half Wood"/></relation>
</osm>"#;

    #[test]
    fn incomplete_wood_is_reported_and_fetched_rest_completes_it() {
        assert_eq!(incomplete(PART_WOOD), vec![900]);
        let trees = |m: &Map| m.props.iter().filter(|q| q.asset.contains("tree")).count();
        let (m, r) = import_cover(PART_WOOD, &[], None, Options::default()).unwrap();
        assert!(r.warnings.iter().any(|w| w.contains("Half Wood")), "{:?}", r.warnings);
        assert!(trees(&m) <= 4, "без второй половины леса нет: {}", trees(&m));
        let (m, r) = import_cover(PART_WOOD, &[PART_WOOD_REST.to_owned()], None, Options::default()).unwrap();
        assert!(!r.warnings.iter().any(|w| w.contains("неполные")), "{:?}", r.warnings);
        assert!(trees(&m) >= 15, "докачанный лес засажен: {}", trees(&m));
    }

    /// Река из двух кусков с общим краем уходит за края карты; через неё пешеходный мост.
    const RIVER: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<osm version="0.6">
 <bounds minlat="51.5300" minlon="-0.1250" maxlat="51.5304" maxlon="-0.1241"/>
 <node id="1" lat="51.53010" lon="-0.1252"/>
 <node id="2" lat="51.53010" lon="-0.12455"/>
 <node id="3" lat="51.53030" lon="-0.12455"/>
 <node id="4" lat="51.53030" lon="-0.1252"/>
 <node id="5" lat="51.53010" lon="-0.1239"/>
 <node id="6" lat="51.53030" lon="-0.1239"/>
 <way id="10"><nd ref="1"/><nd ref="2"/><nd ref="3"/><nd ref="4"/><nd ref="1"/><tag k="natural" v="water"/><tag k="water" v="river"/></way>
 <way id="11"><nd ref="2"/><nd ref="5"/><nd ref="6"/><nd ref="3"/><nd ref="2"/><tag k="natural" v="water"/><tag k="water" v="river"/></way>
 <node id="20" lat="51.5304" lon="-0.1247"/>
 <node id="21" lat="51.5300" lon="-0.1247"/>
 <way id="30"><nd ref="20"/><nd ref="21"/><tag k="highway" v="footway"/><tag k="bridge" v="yes"/><tag k="layer" v="1"/></way>
 <node id="40" lat="51.5304" lon="-0.1243"/>
 <node id="41" lat="51.5300" lon="-0.1243"/>
 <way id="50"><nd ref="40"/><nd ref="41"/><tag k="highway" v="footway"/><tag k="ford" v="stepping_stones"/></way>
</osm>"#;

    #[test]
    fn river_in_parts_has_no_shoal_at_the_join_and_bridge_has_deck_and_rails() {
        let (m, _) = import(RIVER, Options::default()).unwrap();
        let key = |x: i32, y: i32| match m.floor_at(x, y) {
            0 => String::new(),
            k => match &m.materials[k as usize - 1].paint {
                Paint::Texture(t) => t.clone(),
                Paint::Color(_) => "color".into(),
            },
        };
        // Середина русла (y ≈ 14,5): у стыка кусков (x ≈ 20,4) и у краёв карты — глубоко.
        for x in [0, 19, 20, 21, 40] {
            assert!(key(x, 14).contains("water_deep"), "x = {x}: {}", key(x, 14));
        }
        // У берега (около y = 8) — мелко.
        assert!((7..=10).any(|y| key(20, y).contains("water_shallow")), "{:?}", (7..=10).map(|y| key(20, y)).collect::<Vec<_>>());
        // Вдоль шва кусков (x = 20) от берега до берега — вода, без полосы суши
        // и без выемок на берегах.
        for y in 9..=19 {
            assert!(key(20, y).contains("water"), "шов, y = {y}: {}", key(20, y));
        }
        let deck = m.paths.iter().find(|p| matches!(&m.materials[p.material].paint, Paint::Texture(t) if t.contains("wood_planks"))).expect("настил");
        assert!(deck.along);
        // Две точки OSM — дорога растаяла бы целиком: точки не реже двух клеток.
        assert!(deck.points.windows(2).all(|s| length(s[0], s[1]) <= 2.01), "{:?}", deck.points);
        assert_eq!(m.walls.iter().filter(|w| w.low).count(), 2, "перила");
        // Камни-переходы — цепочка камней, а не полоса тропы через воду; полос две:
        // подложка моста и настил над ней.
        assert_eq!(m.paths.len(), 2, "лишняя полоса");
        assert!(matches!(m.materials[m.paths[0].material].paint, Paint::Color(_)), "подложка не снизу");
        let stones = m.props.iter().filter(|q| q.asset.contains("boulder")).count();
        assert!((25..=40).contains(&stones), "камней {stones}");
        // Мостки по болоту лежат на земле — без перил: с них сходят в топь.
        let boardwalk = RIVER.replace(r#"<tag k="bridge" v="yes"/>"#, r#"<tag k="bridge" v="boardwalk"/>"#);
        let (m, _) = import(&boardwalk, Options::default()).unwrap();
        assert_eq!(m.walls.iter().filter(|w| w.low).count(), 0, "перила у мостков");
        assert!(m.paths.iter().any(|p| p.along), "настил мостков");
    }

    #[test]
    fn same_file_same_map() {
        assert_eq!(sample().0.to_json(), sample().0.to_json());
    }

    #[test]
    fn big_area_is_scaled_down_with_warning() {
        let (m, r) = import(SAMPLE, Options { max_cells: 20, ..Options::default() }).unwrap();
        assert!(m.width <= 20 && m.height <= 20);
        assert!(r.warnings.iter().any(|w| w.contains("клетка")), "{:?}", r.warnings);
    }

    #[test]
    fn not_osm_is_an_error() {
        assert!(import("<html></html>", Options::default()).is_err());
        assert!(import("не xml", Options::default()).is_err());
    }

    #[test]
    fn multipolygon_rings_join_and_courtyard_stays_open() {
        let r = assemble(vec![vec![1, 2, 3], vec![3, 4, 1]]);
        assert!(r.len() == 1 && r[0].len() == 5 && r[0].first() == r[0].last(), "{r:?}");
        assert_eq!(assemble(vec![vec![1, 2, 3], vec![1, 4, 3]]).len(), 1);
        assert!(assemble(vec![vec![1, 2, 3]]).is_empty(), "незамкнутое отбрасывается");
        let mk = Mask::for_cells(10, 10);
        let outer = vec![[1.0, 1.0], [9.0, 1.0], [9.0, 9.0], [1.0, 9.0]];
        let inner = vec![[4.0, 4.0], [6.0, 4.0], [6.0, 6.0], [4.0, 6.0]];
        let cov = coverage(&mk, &[outer, inner], 0.0);
        let at = |x: f32, y: f32| {
            let k = (y * mask::RES as f32) as usize * mk.w + (x * mask::RES as f32) as usize;
            cov.iter().find(|c| c.0 == k).map_or(0.0, |c| c.1)
        };
        assert_eq!(at(2.0, 2.0), 1.0);
        assert_eq!(at(5.0, 5.0), 0.0, "двор внутри дома залит");
        assert_eq!(at(0.5, 0.5), 0.0);
    }

    /// Настоящая выгрузка: `KARTOGRAF_OSM=путь.osm cargo test osm_file -- --ignored`.
    /// Рядом с файлом кладёт `.karta` и грубый снимок `_preview.png`.
    #[test]
    #[ignore]
    fn osm_file() {
        let Ok(path) = std::env::var("KARTOGRAF_OSM") else { return };
        let xml = std::fs::read_to_string(&path).unwrap();
        let t = std::time::Instant::now();
        let (m, r) = import(&xml, Options::default()).unwrap();
        println!("{}×{} клеток, домов {}, дорог {}, стен {}, дверей {}, объектов {}, подписей {}, {:?}", m.width, m.height, r.count, m.paths.len(), m.walls.len(), m.portals.len(), m.props.len(), m.labels.len(), t.elapsed());
        println!("предупреждения: {:?}", r.warnings);
        let stem = std::path::Path::new(&path).with_extension("");
        std::fs::write(stem.with_extension("karta"), m.to_json()).unwrap();
        preview(&m, 4).save(format!("{}_preview.png", stem.display())).unwrap();
    }

    /// Все пути ассетов из паков настроек в файл: `DUMP_KEYS=путь cargo test fa_keys -- --ignored`.
    #[test]
    #[ignore]
    fn fa_keys() {
        let Ok(out) = std::env::var("DUMP_KEYS") else { return };
        let settings = std::path::PathBuf::from(std::env::var("APPDATA").unwrap()).join("Kartograf/settings.json");
        let s: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(settings).unwrap()).unwrap();
        let mut lines = Vec::new();
        for p in s["packs"].as_array().unwrap() {
            if let Ok(pack) = crate::pck::read_index(std::path::Path::new(p.as_str().unwrap())) {
                lines.extend(pack.entries.iter().map(|e| format!("{}\t{}", pack.name, e.path)));
            }
        }
        std::fs::write(out, lines.join("\n")).unwrap();
    }

    /// Что подбор берёт в слоты из всех паков настроек (полы тоже):
    /// `SLOTS=meadow,shingle cargo test fa_slots -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn fa_slots() {
        let Ok(slots) = std::env::var("SLOTS") else { return };
        let settings = std::path::PathBuf::from(std::env::var("APPDATA").unwrap()).join("Kartograf/settings.json");
        let s: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(settings).unwrap()).unwrap();
        let mut assets = Vec::new();
        for p in s["packs"].as_array().unwrap() {
            if let Ok(a) = crate::library::pack_assets(std::path::Path::new(p.as_str().unwrap())) {
                assets.extend(a);
            }
        }
        let pal = Palette::from_library(&assets, Box::new(crate::palette::fake_measure));
        for slot in slots.split(',') {
            let keys = pal.keys(slot);
            println!("== {slot}: {}", keys.len());
            for k in keys.iter().take(30) {
                let name = assets.iter().find(|a| &a.key == k).map_or("?", |a| a.name.as_str());
                println!("   {} | {name}", k.rsplit('/').next().unwrap_or(k));
            }
        }
    }

    /// Паки из настроек программы: `FIND=rail,track cargo test find_fa -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn find_fa() {
        let Ok(words) = std::env::var("FIND") else { return };
        let settings = std::path::PathBuf::from(std::env::var("APPDATA").unwrap()).join("Kartograf/settings.json");
        let s: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(settings).unwrap()).unwrap();
        let mut assets = Vec::new();
        for p in s["packs"].as_array().unwrap() {
            if let Ok(a) = crate::library::pack_assets(std::path::Path::new(p.as_str().unwrap())) {
                assets.extend(a);
            }
        }
        println!("ассетов: {}", assets.len());
        // `DUMP=папка` — найденные текстуры пола (`terrain/`) уменьшенными картинками, по 20 на слово.
        if let Ok(dir) = std::env::var("DUMP") {
            for w in words.split(',') {
                let mut n = 0;
                for p in s["packs"].as_array().unwrap() {
                    let Ok(pack) = crate::pck::read_index(std::path::Path::new(p.as_str().unwrap())) else { continue };
                    for e in pack.entries.iter().filter(|e| (e.path.contains("terrain/") || e.path.contains("patterns/")) && e.path.to_lowercase().contains(w)) {
                        if n >= 120 {
                            break;
                        }
                        let Ok(img) = crate::pck::read_bytes(&pack.file, e.offset, e.size).and_then(|b| image::load_from_memory(&b).map_err(|x| x.to_string())) else { continue };
                        let name = e.path.rsplit('/').next().unwrap_or("x").replace(".webp", ".png");
                        img.thumbnail(256, 256).save(std::path::Path::new(&dir).join(name)).unwrap();
                        n += 1;
                    }
                }
            }
        }
        for w in words.split(',') {
            // `FLOORS=1` — только текстуры пола, без объектов.
            let floors_only = std::env::var("FLOORS").is_ok();
            let mut hits: Vec<String> = assets
                .iter()
                .filter(|a| a.key.to_lowercase().contains(w) && (!floors_only || a.kind == crate::library::Kind::Floor))
                .map(|a| a.key.rsplit('|').next().unwrap_or(&a.key).to_owned())
                .collect();
            hits.sort();
            hits.dedup_by(|a, b| a.rsplit('/').next().unwrap().split('_').take(3).eq(b.rsplit('/').next().unwrap().split('_').take(3)));
            println!("== {w}: {}", hits.len());
            for h in hits.iter().take(30) {
                println!("   {h}");
            }
        }
    }

    /// Грубый снимок для глаз: цвет пола по имени текстуры, дороги, стены, двери, деревья.
    fn preview(m: &Map, ppc: u32) -> image::RgbImage {
        let color = |mat: u8| -> [u8; 3] {
            let key = match &m.materials[mat as usize].paint {
                Paint::Texture(k) => k.as_str(),
                Paint::Color(c) => return *c,
            };
            let table: [(&str, [u8; 3]); 16] = [
                ("cobblestone", [104, 98, 92]),
                ("stone_tiles", [178, 172, 160]),
                ("flagstones", [148, 142, 132]),
                ("grass_dry", [150, 150, 84]),
                ("grass", [96, 138, 72]),
                ("water_deep", [40, 74, 124]),
                ("water", [82, 128, 168]),
                ("gravel", [150, 140, 124]),
                ("wood_planks", [168, 128, 88]),
                ("marble", [220, 218, 212]),
                ("dirt", [130, 100, 70]),
                ("forest", [70, 88, 50]),
                ("swamp", [70, 96, 80]),
                ("rocky", [120, 116, 112]),
                ("sand", [214, 196, 140]),
                ("mud", [98, 80, 60]),
            ];
            table.iter().find(|(k, _)| key.contains(k)).map_or([255, 0, 255], |t| t.1)
        };
        let (w, h) = (m.width * ppc, m.height * ppc);
        let mut img = image::RgbImage::new(w, h);
        let res = mask::RES as f32;
        let top = |coats: &[crate::map::Coat], i: isize, j: isize| coats.iter().rev().find(|c| c.mask.get(i, j) >= 128).map(|c| c.material);
        for y in 0..h {
            for x in 0..w {
                let (i, j) = (((x as f32 + 0.5) / ppc as f32 * res) as isize, ((y as f32 + 0.5) / ppc as f32 * res) as isize);
                let c = if m.rooms.get(i, j) >= 128 { top(&m.room_floor, i, j) } else { top(&m.terrain, i, j) };
                img.put_pixel(x, y, image::Rgb(c.map_or([0, 0, 0], color)));
            }
        }
        let dot = |img: &mut image::RgbImage, p: [f32; 2], r: f32, c: [u8; 3]| {
            let (cx, cy, rp) = (p[0] * ppc as f32, p[1] * ppc as f32, (r * ppc as f32).max(0.5));
            for y in (cy - rp).floor() as i32..=(cy + rp).ceil() as i32 {
                for x in (cx - rp).floor() as i32..=(cx + rp).ceil() as i32 {
                    if x >= 0 && y >= 0 && (x as u32) < w && (y as u32) < h && ((x as f32 - cx).powi(2) + (y as f32 - cy).powi(2)) <= rp * rp {
                        img.put_pixel(x as u32, y as u32, image::Rgb(c));
                    }
                }
            }
        };
        let line = |img: &mut image::RgbImage, a: [f32; 2], b: [f32; 2], r: f32, c: [u8; 3]| {
            let steps = ((length(a, b) * ppc as f32) as usize).max(1);
            for s in 0..=steps {
                let f = s as f32 / steps as f32;
                dot(img, [a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f], r, c);
            }
        };
        for p in &m.paths {
            let c = color(p.material as u8);
            for s in p.curve().windows(2) {
                line(&mut img, s[0], s[1], p.width / 2.0, c);
            }
        }
        for s in crate::map::cut_portals(&m.room_edges(), &m.portals) {
            line(&mut img, s.0, s.1, 0.15, [40, 30, 26]);
        }
        for wl in &m.walls {
            let c = if wl.low { [90, 80, 70] } else { [40, 30, 26] };
            for s in wl.points.windows(2) {
                line(&mut img, s[0], s[1], 0.15, c);
            }
        }
        for p in &m.portals {
            line(&mut img, p.a, p.b, 0.18, [210, 40, 30]);
        }
        // Кроны полупрозрачными кругами: видно и густоту, и что под ними.
        for q in &m.props {
            let (r, c) = if q.asset.contains("tree") {
                (1.4 * q.scale, [34, 80, 30])
            } else if q.asset.contains("bush") {
                (0.5 * q.scale, [120, 170, 70])
            } else if q.asset.contains("boulder") {
                (0.5 * q.scale, [90, 90, 96])
            } else {
                (0.4, [240, 200, 40])
            };
            dot(&mut img, q.pos, r * 0.35, c);
            let (cx, cy, rp) = (q.pos[0] * ppc as f32, q.pos[1] * ppc as f32, r * ppc as f32);
            for y in (cy - rp).floor() as i32..=(cy + rp).ceil() as i32 {
                for x in (cx - rp).floor() as i32..=(cx + rp).ceil() as i32 {
                    let d = ((x as f32 - cx).powi(2) + (y as f32 - cy).powi(2)).sqrt();
                    if x >= 0 && y >= 0 && (x as u32) < w && (y as u32) < h && (d - rp).abs() < 0.7 {
                        img.put_pixel(x as u32, y as u32, image::Rgb(c));
                    }
                }
            }
        }
        img
    }
}
