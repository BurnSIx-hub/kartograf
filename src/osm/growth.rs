//! Растительность площадей, площади и парки.

use super::*;

/// Что растёт на площадном объекте: слот, шаг между посадками в клетках, масштаб
/// (от и до) и доля мест, где правда посажено (вереск — редкие кусты).
pub(super) struct Growth {
    pub(super) slots: &'static [&'static str],
    pub(super) spacing: f32,
    pub(super) scale: (f32, f32),
    pub(super) share: f64,
    /// Подстилка под кронами — у леса.
    pub(super) litter: bool,
    /// Старшинство: контуры в OSM перекрываются (вереск накрывает и лес), и
    /// младший не сажает там, где лежит старший. Лес — 0.
    pub(super) rank: u8,
}

pub(super) fn growth(t: &Tags) -> Option<Growth> {
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
pub(super) fn growth_rock(t: &Tags) -> bool {
    matches!(get(t, "natural"), "bare_rock" | "scree" | "stone")
}

/// Рамка колец, обрезанная по карте; пустая — контур весь за краем.
pub(super) fn clipped_box(rings: &[Vec<[f32; 2]>], w: f32, h: f32) -> Option<([f32; 2], [f32; 2])> {
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
pub(super) fn inside(rings: &[Vec<[f32; 2]>], p: [f32; 2]) -> bool {
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
pub(super) fn plaza(t: &Tags) -> bool {
    get(t, "railway") == "platform" || (get(t, "highway") == "pedestrian" && get(t, "area") == "yes") || get(t, "place") == "square" || t.contains_key("area:highway")
}

/// Парк или сад, чью траву решает только `leisure`: газон и луг у него не
/// отмечены. Такой контур в городе обнимает и дорожки, и мощёные площадки.
pub(super) fn park_only(t: &Tags) -> bool {
    matches!(get(t, "leisure"), "park" | "garden") && get(t, "landuse").is_empty() && get(t, "natural").is_empty()
}

/// Площади, что в OSM «грунт» (`surface=dirt` и подобное), а на снимке выглядят
/// как мощёные площади той же карты, — мостовой самой большой из них: номер
/// контура и текстура. Правка StreetComplete от 24.08.2026, сделанная на глаз
/// (одна правка от Киева до Москвы), положила грунт на Васильевский спуск и
/// Соборную площадь, а на снимке 2025 года там брусчатка. Мерило — мощёное этого
/// же снимка, а не «серость»: светлая земля пустыни на снимке серее асфальта.
pub(super) fn relay_plazas(areas: &[Area], m: &Map, hint: Option<&Hint>, sea: bool, warnings: &mut Vec<String>) -> HashMap<usize, &'static str> {
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
