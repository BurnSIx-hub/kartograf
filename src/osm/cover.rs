//! Подсказка со снимка: классы, кроны, тон; кроны в слоты ассетов.

use super::*;

/// Подсказка со снимка (`tools/osm/cover.py`): что растёт и лежит в каждой точке
/// участка, по рамке выгрузки. В OSM нет одиноких деревьев, орляка по склонам и
/// травы на «голом камне» — снимок их видит. Классы — цветами картинки.
#[derive(Clone, Debug)]
pub struct Cover {
    pub(super) w: usize,
    pub(super) h: usize,
    pub(super) data: Vec<u8>,
    /// Кроны из анализа снимка (`tools/osm/trees.py`, `<имя>.crowns.json`):
    /// место, размер и порода каждого дерева. Нет — ищутся по классам подсказки.
    pub(super) crowns: Option<Vec<Crown>>,
    /// Сам снимок по той же рамке — сверить покрытие OSM с видом сверху. Есть
    /// только у анализа в программе: у `.cover.png` скриптов снимка рядом нет.
    pub(super) photo: Option<image::RgbImage>,
    /// Вода снимка, что кладётся и там, где в OSM её нет ([`photo_water`]), — по
    /// пикселям снимка. Есть только вне Европы, где снимок сверен с покровом Esri:
    /// в Амазонии в OSM пусто, и озеро на снимке выходило сушей.
    pub(super) wild: Option<Vec<bool>>,
}

/// Вид участка на снимке: средняя яркость и насыщенность (размах каналов), 0..255.
#[derive(Clone, Copy, Debug)]
pub(super) struct Tone {
    pub(super) value: f32,
    pub(super) chroma: f32,
}

/// Крона из `<имя>.crowns.json`: центр в долях рамки, радиус в метрах, порода.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct Crown {
    pub(super) u: f32,
    pub(super) v: f32,
    pub(super) r: f32,
    /// `broad`, `conifer` или `bush`.
    #[serde(default)]
    pub(super) kind: String,
    /// Откуда порода: `nfi:<тип леса>` — решена по карте лесов; `none` — решает
    /// импорт по тегу леса OSM и очереди.
    #[serde(default)]
    pub(super) src: String,
    /// Место в очереди на хвойную, 0..1: в лесу с долей хвойных `p` хвойные — кроны
    /// с очередью меньше `p` (тёмные и холодные на снимке, куртинами).
    #[serde(default)]
    pub(super) queue: f32,
}

impl Crown {
    /// Крона из анализа снимка в программе (`place.rs`).
    pub fn new(u: f32, v: f32, r: f32, kind: String, src: String, queue: f32) -> Self {
        Self { u, v, r, kind, src, queue }
    }
}

#[derive(serde::Deserialize)]
pub(super) struct CrownFile {
    pub(super) crowns: Vec<Crown>,
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
    pub(super) const COLORS: [([u8; 3], Class); 10] = [
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
        Ok(Self { w, h, data, crowns: None, photo: None, wild: None })
    }

    /// Из классов анализа снимка в программе (`place.rs`): номер класса — место
    /// в `COLORS`, по строкам.
    pub fn from_classes(w: usize, h: usize, data: Vec<u8>, crowns: Vec<Crown>) -> Self {
        Self { w, h, data, crowns: Some(crowns), photo: None, wild: None }
    }

    /// С водой снимка там, где в OSM её нет: маска по пикселям, как `data`.
    pub fn with_wild_water(mut self, mask: Vec<bool>) -> Self {
        if mask.len() == self.w * self.h {
            self.wild = Some(mask);
        }
        self
    }

    /// Доля такой воды в круге — как [`Cover::share`].
    pub(super) fn wild_share(&self, u: f32, v: f32, ru: f32, rv: f32) -> f32 {
        let Some(mask) = &self.wild else { return 0.0 };
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
                hit += mask[y as usize * self.w + x as usize] as u32;
            }
        }
        if all == 0 { 0.0 } else { hit as f32 / all as f32 }
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
    pub(super) fn grid(w: usize, h: usize, at: impl Fn(usize, usize) -> Class) -> Self {
        let data = (0..w * h).map(|k| Self::COLORS.iter().position(|c| c.1 == at(k % w, k / w)).unwrap_or(0) as u8).collect();
        Self { w, h, data, crowns: None, photo: None, wild: None }
    }

    /// Доля класса в круге радиуса `r` вокруг точки; `u`, `v`, `r` — в долях рамки по x и y.
    pub(super) fn share(&self, u: f32, v: f32, ru: f32, rv: f32, class: Class) -> f32 {
        self.share_of(u, v, ru, rv, |c| c == class)
    }

    /// Доля классов, что подходят под `pick`, в круге — как [`Cover::share`].
    pub(super) fn share_of(&self, u: f32, v: f32, ru: f32, rv: f32, pick: impl Fn(Class) -> bool) -> f32 {
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
    pub(super) fn tone_at(&self, u: f32, v: f32) -> Option<Tone> {
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
    pub(super) fn land_share(&self, class: Class) -> f32 {
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
    pub(super) fn base_floor(&self) -> &'static str {
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
    pub(super) fn at(&self, u: f32, v: f32) -> Option<Class> {
        let (x, y) = ((u * self.w as f32).floor(), (v * self.h as f32).floor());
        if x < 0.0 || y < 0.0 || x as usize >= self.w || y as usize >= self.h {
            return None;
        }
        Self::COLORS.get(self.data[y as usize * self.w + x as usize] as usize).map(|c| c.1)
    }
}

/// Подсказка, приложенная к карте: точки — в клетках карты.
pub(super) struct Hint<'a> {
    pub(super) cover: &'a Cover,
    /// Ширина и высота рамки выгрузки в клетках (карта — с округлением вверх).
    pub(super) span: [f32; 2],
    /// Наклон снимка ([`photo_lean`]): на сколько клеток крыша на снимке сдвинута
    /// от контура на клетку высоты (x — восток, y — юг). Нет — не найден.
    pub(super) lean: Option<[f32; 2]>,
    /// Куда в плане светит солнце снимка ([`photo_sun`]): единичный вектор к нему.
    pub(super) sun: Option<[f32; 2]>,
}

impl Hint<'_> {
    /// Сдвиг на снимке того, что на высоте `h` клеток.
    pub(super) fn shift(&self, h: f32) -> [f32; 2] {
        self.lean.map_or([0.0, 0.0], |l| [l[0] * h, l[1] * h])
    }

    /// Яркость снимка в точке `p` (клетки), между пикселями — по четырём соседним.
    pub(super) fn luma_at(&self, p: [f32; 2]) -> Option<f32> {
        let photo = self.cover.photo.as_ref()?;
        let (w, h) = (photo.width() as f32, photo.height() as f32);
        let (x, y) = (p[0] / self.span[0] * w - 0.5, p[1] / self.span[1] * h - 0.5);
        if x < 0.0 || y < 0.0 || x >= w - 1.0 || y >= h - 1.0 {
            return None;
        }
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let at = |dx: u32, dy: u32| luma(photo.get_pixel(x0 as u32 + dx, y0 as u32 + dy).0);
        Some((at(0, 0) * (1.0 - fx) + at(1, 0) * fx) * (1.0 - fy) + (at(0, 1) * (1.0 - fx) + at(1, 1) * fx) * fy)
    }

    /// Доля класса в круге радиуса `r` клеток вокруг `p`.
    pub(super) fn share(&self, p: [f32; 2], r: f32, class: Class) -> f32 {
        self.cover.share(p[0] / self.span[0], p[1] / self.span[1], r / self.span[0], r / self.span[1], class)
    }

    pub(super) fn at(&self, p: [f32; 2]) -> Option<Class> {
        self.cover.at(p[0] / self.span[0], p[1] / self.span[1])
    }

    /// Зелень на снимке в круге радиуса `r` клеток: доля травы, крон, бурого и
    /// сухого среди всего, что не вода (тень дома и кроны снимок путает с водой).
    /// Одна вода — не видно, 1.
    pub(super) fn green(&self, p: [f32; 2], r: f32) -> f32 {
        let (u, v, ru, rv) = (p[0] / self.span[0], p[1] / self.span[1], r / self.span[0], r / self.span[1]);
        let green = self.cover.share_of(u, v, ru, rv, |c| matches!(c, Class::Grass | Class::Trees | Class::Brown | Class::Dry | Class::Bog));
        let seen = 1.0 - self.cover.share_of(u, v, ru, rv, |c| c == Class::Water);
        if seen < 0.2 { 1.0 } else { (green / seen).min(1.0) }
    }

    /// Средний вид сэмплов маски (`k` — номер сэмпла, вес больше половины) на
    /// снимке; `mw` — ширина маски. Меньше 50 сэмплов на снимке — не судить.
    pub(super) fn tone(&self, samples: &[(usize, f32)], mw: usize) -> Option<Tone> {
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

    /// Цвет снимка по сэмплам маски, как у [`Hint::tone`]: срединный по каждому
    /// каналу — блик стекла и тень трубы его не сдвигают. `shift` — сдвиг на
    /// снимке в клетках: крыша высокого дома лежит на нём не над контуром.
    /// `sunlit` — без тени: точки темнее половины верхней четверти яркости не в
    /// счёт, если их меньше половины. Для цвета кровли так нельзя — тёмная жесть
    /// рядом со светлым фонарём тоже «тень» (тёмная плоская кровля у Никольской
    /// выходила светло-бирюзовой), — а светлую кровлю в тени крыла узнать можно.
    pub(super) fn colour(&self, samples: &[(usize, f32)], mw: usize, shift: [f32; 2], sunlit: bool) -> Option<[u8; 3]> {
        let photo = self.cover.photo.as_ref()?;
        let res = mask::RES as f32;
        let mut px: Vec<[u8; 3]> = Vec::new();
        for &(k, v) in samples {
            if v < 0.5 {
                continue;
            }
            let p = [((k % mw) as f32 + 0.5) / res + shift[0], ((k / mw) as f32 + 0.5) / res + shift[1]];
            let (x, y) = ((p[0] / self.span[0] * photo.width() as f32).floor(), (p[1] / self.span[1] * photo.height() as f32).floor());
            if x < 0.0 || y < 0.0 || x >= photo.width() as f32 || y >= photo.height() as f32 {
                continue;
            }
            px.push(photo.get_pixel(x as u32, y as u32).0);
        }
        if px.len() < 50 {
            return None;
        }
        let mut lumas: Vec<f32> = px.iter().map(|&c| luma(c)).collect();
        let q = lumas.len() * 3 / 4;
        let bright = *lumas.select_nth_unstable_by(q, f32::total_cmp).1;
        let lit: Vec<[u8; 3]> = px.iter().copied().filter(|&c| luma(c) >= 0.5 * bright).collect();
        let pick = if sunlit && lit.len() * 2 >= px.len() { lit } else { px };
        let mut seen: [Vec<u8>; 3] = std::array::from_fn(|c| pick.iter().map(|p| p[c]).collect());
        Some(std::array::from_fn(|c| {
            let v = &mut seen[c];
            let mid = v.len() / 2;
            *v.select_nth_unstable(mid).1
        }))
    }

    /// Деревья снимка в клетках карты `w`×`h`: из анализа (`crowns.json`), а без
    /// него — найденные по классам ([`Hint::crowns`]); порода тогда не решена, а
    /// очередь на хвойную — куртинами ~25 м.
    pub(super) fn trees(&self, w: usize, h: usize, cell_m: f32) -> Vec<Spot> {
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
    pub(super) fn crowns(&self, w: usize, h: usize) -> Vec<([f32; 2], f32)> {
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
pub(super) struct Spot {
    pub(super) q: [f32; 2],
    pub(super) r: f32,
    pub(super) conifer: Option<bool>,
    pub(super) queue: f32,
}

/// Слот дерева со снимка: крона `r` клеток, порода по карте лесов (`conifer`),
/// лес OSM под ним и место в очереди на хвойную. Карта лесов главнее тега OSM.
/// В сухом краю (`arid`) лиственное — каменный дуб, куст — сухой.
pub(super) fn crown_slot(r: f32, conifer: Option<bool>, wood: Option<&Growth>, queue: f32, arid: bool) -> &'static str {
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
pub(super) fn arid_slot(slot: &'static str, arid: bool) -> &'static str {
    if arid && matches!(slot, "bush_summer" | "bush" | "fern") { "matorral" } else { slot }
}

/// Слот хвойной кроны радиуса `r` клеток: узкая — ель, широкая — сосна.
pub(super) fn conifer_slot(r: f32) -> &'static str {
    if r < 2.2 { "fir" } else { "pine" }
}

/// Доля хвойных в лесу OSM по его слотам деревьев (`leaf_type`: хвойный — все,
/// лиственный — ни одного, смешанный и без тега — по слотам).
pub(super) fn conifer_share(g: &Growth) -> f32 {
    let trees: Vec<&&str> = g.slots.iter().filter(|s| s.starts_with("tree") || **s == "pine").collect();
    if trees.is_empty() { 0.0 } else { trees.iter().filter(|s| ***s == "pine").count() as f32 / trees.len() as f32 }
}

/// Кроны сплошного полога, клетки радиуса: в зрелом лесу кроны 12–18 м.
pub(super) const CROWN_CANOPY: (f32, f32) = (4.0, 6.0);
/// Самая широкая одиночная крона, клетки радиуса: дуб или бук у аббатства — 20 м.
pub(super) const CROWN_LONE: f32 = 7.0;
/// Соседние кроны стоят не ближе этой доли суммы радиусов: случайная укладка
/// кругов рыхлее сотовой, и при 0,8 в пологе Болтона оставались просветы.
pub(super) const CROWN_OVERLAP: f32 = 0.75;
/// Крона на карте шире круга, вписанного в пятно на снимке: пятно не круглое.
pub(super) const CROWN_GROW: f32 = 1.2;
/// Крона уже — куст, клетки радиуса.
pub(super) const CROWN_BUSH: f32 = 1.3;
/// Самая узкая крона по снимку, клетки радиуса: cover.py снимает всё уже 3 м.
pub(super) const CROWN_MIN: f32 = 0.9;

/// Расстояние от каждой точки маски до ближайшей точки вне её, в точках сетки
/// (точный евклидов, Фельценшваб — Хуттенлохер). Край сетки краем не считается.
pub(super) fn distance_inside(mask: &[bool], w: usize, h: usize) -> Vec<f32> {
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
pub(super) fn drop_crown(m: &mut Map, slot: &str, q: [f32; 2], rot: f32, r: f32) {
    let pick = asset(slot);
    let across = pick.visible.map_or(0.85 * pick.size[0].max(pick.size[1]), |[l, t, rr, b]| 0.93 * (rr - l).max(b - t));
    drop_as(m, slot, &pick, q, rot, 2.0 * r / across.max(0.1));
}
