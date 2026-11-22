//! Пипетка (I). Щелчок берёт с карты то, что под курсором.
//! «Текстура» — как щелчок по этой картинке на полке: покрытие крыши, объект, дверь,
//! вид стены, пол — и инструмент, который ими рисует.
//! «Цвет» — цвет окна под курсором: им перекрашивается картинка инструмента, с
//! которого взяли пипетку (или выбранного), узор и светотень остаются (`tint:`).

use super::*;

/// Сторона квадрата точек снимка, по которому берётся цвет: срединное по каждому
/// каналу — линия сетки или обводка в одну-две точки его не сдвигают.
const SAMPLE: usize = 5;

/// Метка снимка окна, заказанного пипеткой: его не перехватят выгрузка и самопроверка.
pub(super) struct PickShot;

/// Что перекрасит пипетка в режиме «Цвет».
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Target {
    Label,
    Roof,
    Wall,
    Door,
    Object,
    Scatter,
    Floor,
}

impl Target {
    pub(super) fn label(self) -> &'static str {
        match self {
            Target::Label => "подпись",
            Target::Roof => "покрытие крыши",
            Target::Wall => "стены",
            Target::Door => "дверь",
            Target::Object => "объект",
            Target::Scatter => "набор разброса",
            Target::Floor => "пол",
        }
    }
}

/// Срединный цвет квадрата `SAMPLE`×`SAMPLE` точек снимка вокруг `at` (точки снимка).
pub(super) fn sample_color(image: &egui::ColorImage, at: [f32; 2]) -> Option<[u8; 3]> {
    let [w, h] = image.size;
    let (cx, cy) = (at[0].floor(), at[1].floor());
    if cx < 0.0 || cy < 0.0 || cx >= w as f32 || cy >= h as f32 {
        return None;
    }
    let (cx, cy, r) = (cx as usize, cy as usize, SAMPLE / 2);
    let mut ch: [Vec<u8>; 3] = Default::default();
    for y in cy.saturating_sub(r)..=(cy + r).min(h - 1) {
        for x in cx.saturating_sub(r)..=(cx + r).min(w - 1) {
            let c = image.pixels[y * w + x];
            ch[0].push(c.r());
            ch[1].push(c.g());
            ch[2].push(c.b());
        }
    }
    Some(ch.map(|mut v| {
        v.sort_unstable();
        v[v.len() / 2]
    }))
}

/// Картинка в цвете `c`: узор и светотень остаются; сплошной цвет становится этим.
pub(super) fn recolored(paint: &Paint, c: [u8; 3]) -> Paint {
    match paint {
        Paint::Texture(k) => Paint::Texture(library::tint_key(c, k)),
        Paint::Color(_) => Paint::Color(c),
    }
}

/// Имя материала в цвете: прежний цвет из имени уходит.
fn colored_name(name: &str, hex: &str) -> String {
    let base = name.rsplit_once(" #").filter(|(_, h)| h.len() == 6 && h.chars().all(|c| c.is_ascii_hexdigit())).map_or(name, |(b, _)| b);
    format!("{base} {hex}")
}

impl App {
    /// Пипетка ждёт снимок окна (самопроверка ждёт с ней).
    pub fn pick_busy(&self) -> bool {
        self.pick_shot.is_some()
    }

    /// Пипетка сделала своё: инструмент `to`, «прежним» станет тот, с которого её
    /// взяли (или его прежний, если вернулись к нему же) — это решает [`App::set_tool`].
    pub(super) fn leave_pick(&mut self, to: Tool) {
        self.set_tool(to);
    }

    /// Инструмент, для которого решают, видны ли крыши: у пипетки — тот, с которого её взяли.
    pub(super) fn view_tool(&self) -> Tool {
        if self.tool == Tool::Pick { self.pick_from } else { self.tool }
    }

    /// Строка над картой: что брать и что перекрасится.
    pub(super) fn pick_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.pick_color, false, "Текстура");
            ui.selectable_value(&mut self.pick_color, true, "Цвет");
        });
        if self.pick_color {
            match self.color_target() {
                Some(t) => ui.weak(format!("Перекрасится: {}", t.label())),
                None => ui.weak("Перекрашивать нечего: возьмите пипетку с инструмента с картинкой или выберите, что перекрасить"),
            };
        }
    }

    /// Щелчок пипеткой по точке экрана `p`.
    pub(super) fn pick_click(&mut self, ctx: &egui::Context, p: Pos2) {
        if self.pick_color {
            self.pick_shot = Some(p);
            ctx.send_viewport_cmd(ViewportCommand::Screenshot(egui::UserData::new(PickShot)));
        } else {
            let w = self.cam.to_world(self.canvas, p);
            self.pick_texture(w);
        }
    }

    /// Пришёл снимок окна, заказанный пипеткой.
    pub(super) fn pick_shot_taken(&mut self, ctx: &egui::Context, image: &egui::ColorImage) {
        let Some(p) = self.pick_shot.take() else { return };
        let ppp = ctx.pixels_per_point();
        match sample_color(image, [p.x * ppp, p.y * ppp]) {
            Some(c) => self.apply_color(c),
            None => self.notice = Some("Пипетка: точка за краем окна".into()),
        }
    }

    /// Картинка того, что под точкой `w`, становится выбранной, как щелчок по ней на полке.
    pub(super) fn pick_texture(&mut self, w: [f32; 2]) {
        let from = self.pick_from;
        let taken = |key: &str| format!("Взято: {}", library::label(key));
        match self.pick_at(w, false) {
            Some(Sel::Roof(i)) => {
                let key = self.map.roofs[i].paint.clone();
                self.notice = Some(taken(&key));
                self.platform_mode = false;
                self.leave_pick(Tool::Roof);
                self.use_key(Kind::Roof, key, false);
            }
            Some(Sel::Outline(i, k)) => {
                let key = self.map.outline_roofs[i].pieces[k].paint.clone();
                self.notice = Some(taken(&key));
                self.platform_mode = false;
                self.leave_pick(Tool::Roof);
                self.use_key(Kind::Roof, key, false);
            }
            Some(Sel::Prop(i)) => {
                let key = self.map.props[i].asset.clone();
                self.notice = Some(taken(&key));
                if from == Tool::Scatter {
                    self.leave_pick(Tool::Scatter);
                    if !self.scatter.set.contains(&key) {
                        self.scatter.set.push(key);
                    }
                } else {
                    self.leave_pick(Tool::Object);
                    self.object_asset = Some(key);
                    self.object_prefab = None;
                }
            }
            Some(Sel::Portal(i)) => {
                let p = self.map.portals[i].clone();
                self.notice = Some(p.asset.as_deref().map_or_else(|| "Взято: проём без картинки".to_owned(), taken));
                self.leave_pick(Tool::Door);
                self.set_door_kind(p.kind);
                let len = ((p.b[0] - p.a[0]).powi(2) + (p.b[1] - p.a[1]).powi(2)).sqrt();
                self.door_width = len.clamp(1.0, 4.0);
                self.door_asset = p.asset;
                self.door_frame = p.frame;
            }
            Some(Sel::Wall(i)) => {
                let st = self.map.walls[i].style.clone().unwrap_or_else(|| self.map.wall_style.clone());
                self.take_wall(st);
            }
            Some(Sel::Edge(key, _)) => {
                let st = self.map.edge_look(key);
                self.take_wall(st);
            }
            Some(Sel::Path(i)) => {
                let m = self.map.paths[i].material;
                self.take_floor(m, Tool::Path);
            }
            _ => match self.map.floor_under(w) {
                Some((Layer::Rooms, m)) => self.take_floor(m, Tool::Dungeon),
                Some((Layer::Floor, m)) => self.take_floor(m, Tool::Floor),
                None => self.notice = Some("Пипетка: здесь пусто".into()),
            },
        }
    }

    /// Вид стены — новым стенам (или выбранной), инструмент — стены или комнаты.
    fn take_wall(&mut self, st: map::WallStyle) {
        self.notice = Some(match &st.paint {
            Paint::Texture(k) => format!("Взято: {}", library::label(k)),
            Paint::Color(_) => "Взято: стена цветом".to_owned(),
        });
        let to = if matches!(self.pick_from, Tool::Wall | Tool::Dungeon) { self.pick_from } else { Tool::Wall };
        self.leave_pick(to);
        self.set_wall_style(st);
    }

    /// Материал пола: им рисуют кисти пола, дорога и заливка рельефа — к той из них, с
    /// которой взяли пипетку; иначе к `natural`.
    fn take_floor(&mut self, m: usize, natural: Tool) {
        let to = if matches!(self.pick_from, Tool::Dungeon | Tool::Floor | Tool::Path | Tool::Relief) { self.pick_from } else { natural };
        self.leave_pick(to);
        self.material = m.min(self.map.materials.len().saturating_sub(1));
        if to == Tool::Relief {
            self.relief_fill = true;
        }
        self.notice = self.map.materials.get(self.material).map(|x| format!("Взято: {}", x.name));
        self.recolor_path();
    }

    /// Что перекрасит цвет пипетки: выбранное «Выбором» или картинку инструмента, с
    /// которого её взяли.
    pub(super) fn color_target(&self) -> Option<Target> {
        let sel = |t: Target| Some(t);
        match self.pick_from {
            Tool::Select => match self.sel {
                Some(Sel::Label(_)) => sel(Target::Label),
                Some(Sel::Roof(_) | Sel::Outline(..)) => sel(Target::Roof),
                Some(Sel::Wall(_) | Sel::Edge(..)) => sel(Target::Wall),
                Some(Sel::Portal(i)) if self.map.portals.get(i).is_some_and(|p| p.asset.is_some()) => sel(Target::Door),
                Some(Sel::Prop(_)) => sel(Target::Object),
                Some(Sel::Path(_)) => sel(Target::Floor),
                _ => None,
            },
            Tool::Text => sel(Target::Label),
            Tool::Roof if !self.platform_mode => sel(Target::Roof),
            Tool::Wall => sel(Target::Wall),
            Tool::Door if self.door_asset.is_some() => sel(Target::Door),
            Tool::Object if self.object_asset.is_some() && self.object_prefab.is_none() => sel(Target::Object),
            Tool::Scatter if !self.scatter.set.is_empty() => sel(Target::Scatter),
            Tool::Dungeon | Tool::Floor | Tool::Path | Tool::Relief if self.material < self.map.materials.len() => sel(Target::Floor),
            _ => None,
        }
    }

    /// Перекрасить в `c` то, что назвал [`App::color_target`], и вернуться к инструменту.
    pub(super) fn apply_color(&mut self, c: [u8; 3]) {
        let hex = format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2]);
        let target = self.color_target();
        self.leave_pick(self.pick_from);
        let Some(target) = target else {
            self.notice = Some(format!("Пипетка взяла {hex}, но перекрашивать нечего: возьмите её с инструмента с картинкой или выберите, что перекрасить"));
            return;
        };
        let tint = |key: &str| library::tint_key(c, key);
        match target {
            Target::Label => match self.sel {
                Some(Sel::Label(i)) if i < self.map.labels.len() => {
                    if self.map.labels[i].color != c {
                        self.checkpoint();
                        self.map.labels[i].color = c;
                    }
                }
                _ => self.label_style.color = c,
            },
            Target::Roof => {
                let outline = matches!(self.sel, Some(Sel::Outline(..)));
                match self.sel {
                    Some(Sel::Roof(i)) if i < self.map.roofs.len() => {
                        let key = tint(&self.map.roofs[i].paint);
                        if self.map.roofs[i].paint != key {
                            self.checkpoint();
                            self.map.roofs[i].paint = key;
                        }
                    }
                    Some(Sel::Outline(i, k)) => {
                        let key = tint(&self.map.outline_roofs[i].pieces[k].paint);
                        if self.map.outline_roofs[i].pieces[k].paint != key {
                            self.checkpoint();
                            self.map.outline_roofs[i].pieces[k].paint = key;
                        }
                    }
                    _ => {}
                }
                // Крыше OSM — только ей: образец новых прямоугольных не трогаем.
                if !outline {
                    self.roof_style.paint = tint(&self.roof_style.paint);
                }
            }
            Target::Wall => {
                let paint = recolored(&self.wall_style().paint, c);
                self.use_wall_paint(paint);
            }
            Target::Door => match self.sel {
                Some(Sel::Portal(i)) if self.tool == Tool::Select => {
                    if let Some(k) = self.map.portals[i].asset.clone() {
                        self.checkpoint();
                        self.map.portals[i].asset = Some(tint(&k));
                    }
                }
                _ => self.door_asset = self.door_asset.as_deref().map(tint),
            },
            Target::Object => match self.sel {
                Some(Sel::Prop(i)) if self.tool == Tool::Select => {
                    let all = if self.picked.contains(&i) { self.picked.clone() } else { vec![i] };
                    self.checkpoint();
                    let n = self.map.props.len();
                    for k in all.into_iter().filter(|&k| k < n) {
                        self.map.props[k].asset = tint(&self.map.props[k].asset);
                    }
                }
                _ => self.object_asset = self.object_asset.as_deref().map(tint),
            },
            Target::Scatter => {
                let set: Vec<String> = self.scatter.set.iter().map(|k| tint(k)).collect();
                self.scatter.set = set;
            }
            Target::Floor => {
                let Some(m) = self.map.materials.get(self.material).cloned() else { return };
                let paint = recolored(&m.paint, c);
                let fresh = Material { name: colored_name(&m.name, &hex), paint, ..m };
                if !self.map.materials.iter().any(|x| x.paint == fresh.paint && x.rot == fresh.rot) {
                    self.checkpoint();
                }
                match self.map.material_index(fresh) {
                    Some(i) => self.material = i,
                    None => self.error = Some("В карте уже 255 материалов — больше не помещается.".into()),
                }
                self.recolor_path();
            }
        }
        self.notice = Some(format!("Пипетка: {} — в цвет {hex}", target.label()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_is_the_median_around_the_point() {
        // Поле 20×10 зелёное, через точку щелчка идёт линия сетки в одну точку.
        let mut img = egui::ColorImage::new([20, 10], vec![Color32::from_rgb(40, 120, 30); 200]);
        for y in 0..10 {
            img.pixels[y * 20 + 10] = Color32::from_rgb(255, 255, 255);
        }
        assert_eq!(sample_color(&img, [10.4, 5.0]), Some([40, 120, 30]), "линия сетки не сдвигает цвет");
        assert_eq!(sample_color(&img, [0.0, 0.0]), Some([40, 120, 30]), "у угла — по тому, что есть");
        assert_eq!(sample_color(&img, [20.0, 5.0]), None);
        assert_eq!(sample_color(&img, [-1.0, 5.0]), None);
    }

    #[test]
    fn recolor_keeps_the_picture_and_replaces_the_old_color() {
        let key = "builtin:grass_green";
        let once = recolored(&Paint::Texture(key.into()), [200, 30, 30]);
        assert_eq!(once, Paint::Texture(format!("tint:c81e1e:{key}")));
        let Paint::Texture(k) = &once else { unreachable!() };
        assert_eq!(recolored(&once, [1, 2, 3]), Paint::Texture(format!("tint:010203:{key}")), "второй цвет заменяет первый: {k}");
        assert_eq!(recolored(&Paint::Color([0, 0, 0]), [9, 8, 7]), Paint::Color([9, 8, 7]));
        assert_eq!(colored_name("Трава", "#c81e1e"), "Трава #c81e1e");
        assert_eq!(colored_name("Трава #c81e1e", "#010203"), "Трава #010203");
        assert_eq!(colored_name("Дом #1", "#010203"), "Дом #1 #010203");
    }
}
