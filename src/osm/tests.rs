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

/// Дом с зелёной крышей и золотым куполом-частью, трибуна и стена: крыша —
/// у дома (основа и купол поверх), у трибуны и стены — нет.
#[test]
fn houses_get_outline_roofs_with_parts_on_top() {
    let xml = town(
        r##"<way id="10"><nd ref="1"/><nd ref="2"/><nd ref="3"/><nd ref="4"/><nd ref="1"/><tag k="building" v="church"/><tag k="roof:colour" v="#2E8B57"/></way>
 <node id="41" lat="51.53032" lon="-0.12485"/>
 <node id="42" lat="51.53032" lon="-0.12475"/>
 <node id="43" lat="51.53024" lon="-0.12475"/>
 <node id="44" lat="51.53024" lon="-0.12485"/>
 <way id="40"><nd ref="41"/><nd ref="42"/><nd ref="43"/><nd ref="44"/><nd ref="41"/><tag k="building:part" v="yes"/><tag k="roof:shape" v="onion"/><tag k="roof:colour" v="gold"/><tag k="height" v="30"/></way>
 <way id="20"><nd ref="5"/><nd ref="6"/><nd ref="7"/><nd ref="8"/><nd ref="5"/><tag k="building" v="grandstand"/></way>"##,
    );
    let (m, _) = import(&xml, Options::default()).unwrap();
    // Дом в углу (900) и церковь; трибуны нет.
    assert_eq!(m.outline_roofs.len(), 2, "{:?}", m.outline_roofs.iter().map(|r| r.pieces.len()).collect::<Vec<_>>());
    let church = m.outline_roofs.iter().find(|r| r.pieces.len() == 2).expect("у церкви нет купола-части");
    assert_eq!(church.pieces[0].form, RoofForm::Pitched);
    assert!(church.pieces[0].paint.starts_with("tint:2e8b57:"), "{}", church.pieces[0].paint);
    assert_eq!(church.pieces[1].form, RoofForm::Dome);
    let gold = tag_roof("gold").unwrap();
    assert!(church.pieces[1].paint.starts_with(&format!("tint:{:02x}{:02x}{:02x}:", gold[0], gold[1], gold[2])), "{}", church.pieces[1].paint);
}

/// «Красный звон»: кровля и две главки одного тега `#617666`. На снимке под
/// главками — бирюза кровли (снимок сдвинут), главки вместе крупнее сдвига,
/// но каждая мельче: цвет главок — из тега, кровли — со снимка. Две грани
/// одной вальмы (общие узлы) каждая тоже мельче, но вместе — одна кровля:
/// им — снимок.
#[test]
fn small_domes_of_one_colour_keep_the_tag_not_the_roof_below() {
    let xml = town(
        r##"<way id="10"><nd ref="1"/><nd ref="2"/><nd ref="3"/><nd ref="4"/><nd ref="1"/><tag k="building" v="church"/><tag k="height" v="15"/><tag k="roof:colour" v="#617666"/></way>
 <node id="41" lat="51.53035" lon="-0.12495"/>
 <node id="42" lat="51.53035" lon="-0.124849"/>
 <node id="43" lat="51.530287" lon="-0.124849"/>
 <node id="44" lat="51.530287" lon="-0.12495"/>
 <node id="51" lat="51.53025" lon="-0.12495"/>
 <node id="52" lat="51.53025" lon="-0.124849"/>
 <node id="53" lat="51.530187" lon="-0.124849"/>
 <node id="54" lat="51.530187" lon="-0.12495"/>
 <way id="40"><nd ref="41"/><nd ref="42"/><nd ref="43"/><nd ref="44"/><nd ref="41"/><tag k="building:part" v="yes"/><tag k="roof:shape" v="onion"/><tag k="roof:colour" v="#617666"/><tag k="height" v="18"/></way>
 <way id="50"><nd ref="51"/><nd ref="52"/><nd ref="53"/><nd ref="54"/><nd ref="51"/><tag k="building:part" v="yes"/><tag k="roof:shape" v="onion"/><tag k="roof:colour" v="#617666"/><tag k="height" v="18"/></way>
 <node id="61" lat="51.5303" lon="-0.12484"/>
 <node id="62" lat="51.5303" lon="-0.124739"/>
 <node id="63" lat="51.530237" lon="-0.124739"/>
 <node id="64" lat="51.530237" lon="-0.12484"/>
 <node id="65" lat="51.5303" lon="-0.124671"/>
 <node id="66" lat="51.530237" lon="-0.124671"/>
 <way id="60"><nd ref="61"/><nd ref="62"/><nd ref="63"/><nd ref="64"/><nd ref="61"/><tag k="building:part" v="yes"/><tag k="roof:shape" v="skillion"/><tag k="roof:direction" v="W"/><tag k="roof:colour" v="#2e8b57"/><tag k="height" v="16"/><tag k="min_height" v="13"/></way>
 <way id="70"><nd ref="62"/><nd ref="65"/><nd ref="66"/><nd ref="63"/><nd ref="62"/><tag k="building:part" v="yes"/><tag k="roof:shape" v="skillion"/><tag k="roof:direction" v="E"/><tag k="roof:colour" v="#2e8b57"/><tag k="height" v="16"/><tag k="min_height" v="13"/></way>"##,
    );
    let teal = Cover::grid(82, 58, |_, _| Class::Earth).with_photo(image::RgbImage::from_pixel(82, 58, image::Rgb([103, 145, 128])));
    let (m, _) = import_cover(&xml, &[], Some(&teal), Options::default()).unwrap();
    let church = m.outline_roofs.iter().find(|r| r.pieces.len() == 5).expect("частей нет");
    assert!(church.pieces[0].paint.starts_with("tint:679180:"), "кровля не со снимка: {}", church.pieces[0].paint);
    for dome in church.pieces.iter().filter(|p| p.form == RoofForm::Dome) {
        assert!(dome.paint.starts_with("tint:617666:"), "главка в цвет кровли под ней: {}", dome.paint);
    }
    for facet in church.pieces.iter().filter(|p| p.form == RoofForm::Shed) {
        assert!(facet.paint.starts_with("tint:679180:"), "грань вальмы не со снимка: {}", facet.paint);
    }
}

/// Навес (`building=roof`): крыша есть, плоская, а под ней ни комнаты, ни стен.
#[test]
fn canopy_has_a_flat_roof_and_no_rooms() {
    let xml = town(r#"<way id="20"><nd ref="5"/><nd ref="6"/><nd ref="7"/><nd ref="8"/><nd ref="5"/><tag k="building" v="roof"/></way>"#);
    let (m, _) = import(&xml, Options::default()).unwrap();
    assert_eq!(m.outline_roofs.len(), 2, "крыши навеса нет");
    let canopy = m.outline_roofs.iter().find(|r| r.bounds().0[0] < 20.0).expect("навес не там");
    assert_eq!(canopy.pieces[0].form, RoofForm::Flat);
    assert_eq!(m.room_at(23, 10), 0, "под навесом комната");
}

#[test]
fn houses_cast_shadows_away_from_the_sun_as_long_as_they_are_tall() {
    let sq = |x0: f32, y0: f32, s: f32| vec![vec![[x0, y0], [x0 + s, y0], [x0 + s, y0 + s], [x0, y0 + s]]];
    let (house, beyond, shed) = (sq(5.0, 5.0, 4.0), sq(-6.0, -6.0, 5.0), sq(14.0, 2.0, 3.0));
    let solids: Vec<Solid> = vec![(&house, 3.0, false), (&beyond, 5.0, false), (&shed, 2.0, true)];
    let (w, h) = (20 * mask::RES, 20 * mask::RES);
    let light = crate::map::Shadows::default().light();
    let out = shadow_mask(&solids, w, h, light);
    let at = |cx: f32, cy: f32| out[(cy * mask::RES as f32) as usize * w + (cx * mask::RES as f32) as usize];
    let full = (HOUSE_SHADE * 255.0).round() as u8;
    assert!(light[0] < 0.0 && light[1] < 0.0, "солнце не слева сверху");
    assert!(at(10.5, 10.5) > full / 2, "у угла дома тени нет: {}", at(10.5, 10.5));
    assert_eq!(at(14.0, 14.0), 0, "тень длиннее дома");
    assert_eq!(at(7.0, 7.0), 0, "тень в доме");
    assert_eq!(at(4.0, 4.0), 0, "тень со стороны солнца");
    assert!(at(0.5, 0.5) > full / 2, "дом за краем тени не дал");
    assert_eq!(at(15.5, 3.5), full, "под навесом не тень");
}

/// Снимок 100×100 клеток (3 пикселя на клетку): земля тёмная, основание дома
/// серое, крыша светлая и сдвинута на `lean × высоту`.
fn leaning_photo(houses: &[([f32; 2], f32, f32)], lean: [f32; 2]) -> Cover {
    let img = image::RgbImage::from_fn(300, 300, |x, y| {
        let p = [(x as f32 + 0.5) / 3.0, (y as f32 + 0.5) / 3.0];
        let within = |o: [f32; 2], side: f32| p[0] >= o[0] && p[0] < o[0] + side && p[1] >= o[1] && p[1] < o[1] + side;
        let roof = houses.iter().any(|&(o, side, h)| within([o[0] + lean[0] * h, o[1] + lean[1] * h], side));
        let base = houses.iter().any(|&(o, side, _)| within(o, side));
        let v = if roof { 200 } else if base { 110 } else { 60 };
        image::Rgb([v, v, v])
    });
    Cover::grid(100, 100, |_, _| Class::Earth).with_photo(img)
}

fn square(o: [f32; 2], side: f32) -> Vec<Vec<[f32; 2]>> {
    vec![vec![o, [o[0] + side, o[1]], [o[0] + side, o[1] + side], [o[0], o[1] + side]]]
}

/// Крыши на снимке сдвинуты на запад-юго-запад, как в Китай-городе: наклон
/// находится по краям крыш. На ровном снимке без домов — не находится.
#[test]
fn photo_lean_is_found_from_roof_edges() {
    let lean = [-0.25, 0.15];
    let houses: Vec<([f32; 2], f32, f32)> = (0..9).map(|i| ([12.0 + (i % 3) as f32 * 28.0, 12.0 + (i / 3) as f32 * 28.0], 12.0, 6.0 + i as f32 * 2.0)).collect();
    let cover = leaning_photo(&houses, lean);
    let hint = Hint { cover: &cover, span: [100.0, 100.0], lean: None, sun: None };
    let rings: Vec<Vec<Vec<[f32; 2]>>> = houses.iter().map(|&(o, side, _)| square(o, side)).collect();
    let pieces: Vec<(&[Vec<[f32; 2]>], f32)> = rings.iter().zip(&houses).map(|(r, h)| (&r[..], h.2)).collect();
    let found = photo_lean(&hint, &pieces).expect("наклон не найден");
    assert!((found[0] - lean[0]).abs() <= 0.03 && (found[1] - lean[1]).abs() <= 0.03, "{found:?}");
    let flat = Cover::grid(100, 100, |_, _| Class::Earth).with_photo(image::RgbImage::from_pixel(300, 300, image::Rgb([90, 90, 90])));
    let hint = Hint { cover: &flat, span: [100.0, 100.0], lean: None, sun: None };
    assert_eq!(photo_lean(&hint, &pieces), None);
}

/// Цвет крыши высокого дома — там, куда её сдвинул наклон, а не над контуром.
#[test]
fn roof_colour_is_taken_where_the_lean_put_it() {
    let houses = [([40.0, 40.0], 12.0, 20.0)];
    let lean = [-0.25, 0.15];
    let cover = leaning_photo(&houses, lean);
    let m = Map::new(100, 100);
    let rings = square(houses[0].0, 12.0);
    let samples = coverage(&m.rooms, &rings, 0.0);
    let flat = Hint { cover: &cover, span: [100.0, 100.0], lean: None, sun: None };
    let leant = Hint { lean: Some(lean), ..flat };
    // 20 клеток высоты — 30 м; крыша 18 м в поперечнике — снимку верить.
    let colour = |h: &Hint| photo_roof(Some(h), &samples, m.rooms.w, 18.3 * 18.3, 20.0 * CELL_M, CELL_M, false);
    assert_eq!(colour(&leant), Some([200, 200, 200]));
    assert_ne!(colour(&flat), Some([200, 200, 200]), "без наклона — та же крыша");
}

/// Вальма под солнцем с юга: южный скат светлее, северный темнее — скатная.
/// Плоская кровля с полосой светлых фонарей у южного края светлее к солнцу
/// тоже, но перепада на стыках скатов у неё нет — не скатная. Ровная — тоже.
#[test]
fn lit_slopes_tell_a_pitched_roof_from_a_flat_one() {
    let (o, size) = ([30.0f32, 30.0f32], [30.0f32, 16.0f32]);
    let rings = vec![vec![o, [o[0] + size[0], o[1]], [o[0] + size[0], o[1] + size[1]], [o[0], o[1] + size[1]]]];
    let noise = |x: u32, y: u32| ((x.wrapping_mul(7919) ^ y.wrapping_mul(104_729)) % 13) as f32 - 6.0;
    let photo = |roof: &dyn Fn([f32; 2]) -> f32| {
        let img = image::RgbImage::from_fn(300, 300, |x, y| {
            let p = [(x as f32 + 0.5) / 3.0, (y as f32 + 0.5) / 3.0];
            let v = if inside(&rings, p) { roof(p) + noise(x, y) } else { 60.0 };
            let v = v.clamp(0.0, 255.0) as u8;
            image::Rgb([v, v, v])
        });
        Cover::grid(100, 100, |_, _| Class::Earth).with_photo(img)
    };
    // Скат — по ближайшей стороне: север, юг, запад, восток.
    let hipped = |p: [f32; 2]| {
        let d = [p[1] - o[1], o[1] + size[1] - p[1], p[0] - o[0], o[0] + size[0] - p[0]];
        let i = (0..4).min_by(|&a, &b| d[a].total_cmp(&d[b])).unwrap();
        [80.0, 160.0, 120.0, 120.0][i]
    };
    let skylights = |p: [f32; 2]| if p[1] > o[1] + size[1] - 4.0 { 175.0 } else { 120.0 };
    let even = |_: [f32; 2]| 120.0;
    let m = Map::new(100, 100);
    let samples = coverage(&m.rooms, &rings, 0.0);
    let sun = [0.0, 1.0];
    let lit = |roof: &dyn Fn([f32; 2]) -> f32| {
        let cover = photo(roof);
        let hint = Hint { cover: &cover, span: [100.0, 100.0], lean: None, sun: Some(sun) };
        roof_light(&hint, &rings, &samples, m.rooms.w, [0.0, 0.0], CELL_M).is_some_and(|l| pitched_by_light(&l, sun))
    };
    assert!(lit(&hipped), "вальма не узнана");
    assert!(!lit(&skylights), "полоса фонарей — скаты");
    assert!(!lit(&even), "ровная кровля — скаты");
}

/// Большой рынок, вокзал, склад без вида крыши — рядами скатов; простой дом
/// того же размера, маленький склад и рынок с видом в тегах — нет.
#[test]
fn big_halls_get_rows_of_ridges() {
    // Большой — 46×31 м (1400 м²), малый — 21×31 м (670 м²).
    let form = |big: bool, tags: &str| {
        let nodes = if big { r#"<nd ref="1"/><nd ref="6"/><nd ref="7"/><nd ref="4"/><nd ref="1"/>"# } else { r#"<nd ref="5"/><nd ref="6"/><nd ref="7"/><nd ref="8"/><nd ref="5"/>"# };
        let xml = town(&format!(r#"<way id="20">{nodes}{tags}</way>"#));
        let (m, _) = import(&xml, Options::default()).unwrap();
        m.outline_roofs.iter().find(|r| r.bounds().0[0] < 20.0).expect("крыши нет").pieces[0].form
    };
    assert_eq!(form(true, r#"<tag k="building" v="marketplace"/>"#), RoofForm::Rows);
    assert_eq!(form(true, r#"<tag k="building" v="yes"/><tag k="amenity" v="marketplace"/><tag k="building:levels" v="2"/>"#), RoofForm::Rows);
    assert_eq!(form(true, r#"<tag k="building" v="warehouse"/>"#), RoofForm::Rows);
    assert_eq!(form(true, r#"<tag k="building" v="train_station"/>"#), RoofForm::Rows);
    assert_eq!(form(true, r#"<tag k="building" v="yes"/>"#), RoofForm::Pitched);
    assert_eq!(form(true, r#"<tag k="building" v="marketplace"/><tag k="roof:shape" v="flat"/>"#), RoofForm::Flat);
    assert_eq!(form(true, r#"<tag k="building" v="industrial"/><tag k="building:levels" v="4"/><tag k="start_date" v="1975"/>"#), RoofForm::Flat);
    assert_eq!(form(false, r#"<tag k="building" v="warehouse"/>"#), RoofForm::Pitched);
}

/// Без вида крыши в OSM: дом 1966 года в четыре этажа — плоский, дом 1890
/// года — скатный, часовня 1994 года — тоже скатная.
#[test]
fn modern_houses_without_roof_shape_are_flat() {
    let form = |tags: &str| {
        let xml = town(&format!(r#"<way id="20"><nd ref="5"/><nd ref="6"/><nd ref="7"/><nd ref="8"/><nd ref="5"/>{tags}</way>"#));
        let (m, _) = import(&xml, Options::default()).unwrap();
        m.outline_roofs.iter().find(|r| r.bounds().0[0] < 20.0).expect("крыши нет").pieces[0].form
    };
    assert_eq!(form(r#"<tag k="building" v="yes"/><tag k="building:levels" v="4"/><tag k="start_date" v="1966"/>"#), RoofForm::Flat);
    assert_eq!(form(r#"<tag k="building" v="yes"/><tag k="building:levels" v="4"/><tag k="start_date" v="1890"/>"#), RoofForm::Pitched);
    assert_eq!(form(r#"<tag k="building" v="chapel"/><tag k="building:levels" v="3"/><tag k="start_date" v="1994"/>"#), RoofForm::Pitched);
    let year = |v: &str| built(&[("start_date".to_owned(), v.to_owned())].into_iter().collect());
    assert_eq!(year("1966-05-01"), Some(1966));
    assert_eq!(year("~1960s"), Some(1960));
    assert_eq!(year("C19"), None);
}

/// Вальма Сената в OSM — грани-части `skillion` со своим `roof:direction`;
/// пассаж ГУМа — свод `round` поперёк.
#[test]
fn skillion_facets_face_their_direction_and_vaults_keep_orientation() {
    let xml = town(
        r##"<way id="10"><nd ref="1"/><nd ref="2"/><nd ref="3"/><nd ref="4"/><nd ref="1"/><tag k="building" v="yes"/></way>
 <node id="41" lat="51.53032" lon="-0.12495"/>
 <node id="42" lat="51.53032" lon="-0.12485"/>
 <node id="43" lat="51.53024" lon="-0.12485"/>
 <node id="44" lat="51.53024" lon="-0.12495"/>
 <node id="45" lat="51.53032" lon="-0.12475"/>
 <node id="46" lat="51.53024" lon="-0.12475"/>
 <way id="40"><nd ref="41"/><nd ref="42"/><nd ref="43"/><nd ref="44"/><nd ref="41"/><tag k="building:part" v="yes"/><tag k="roof:shape" v="skillion"/><tag k="roof:direction" v="W"/><tag k="height" v="25"/><tag k="min_height" v="20"/></way>
 <way id="50"><nd ref="42"/><nd ref="45"/><nd ref="46"/><nd ref="43"/><nd ref="42"/><tag k="building:part" v="yes"/><tag k="roof:shape" v="round"/><tag k="roof:orientation" v="across"/><tag k="height" v="23"/><tag k="roof:height" v="3"/></way>"##,
    );
    let (m, _) = import(&xml, Options::default()).unwrap();
    let roof = m.outline_roofs.iter().find(|r| r.pieces.len() == 3).expect("частей нет");
    let shed = roof.pieces.iter().find(|p| p.form == RoofForm::Shed).expect("грань не односкатная");
    let d = shed.down.expect("у грани нет стороны");
    assert!(d[0] < -0.99 && d[1].abs() < 0.01, "W — не на запад: {d:?}");
    assert!((shed.eaves * CELL_M - 20.0).abs() < 1e-3, "{}", shed.eaves);
    let vault = roof.pieces.iter().find(|p| p.form == RoofForm::Round).expect("свода нет");
    assert!(vault.across);
    assert!((vault.eaves * CELL_M - 20.0).abs() < 1e-3, "{}", vault.eaves);
}

#[test]
fn css_roof_colours_are_muted() {
    let black = tag_roof("black").unwrap();
    assert!(luma(black) >= 25.0, "чёрная — дыра: {black:?}");
    let green = tag_roof("lightgreen").unwrap();
    assert!(luma(green) < luma(colour("lightgreen").unwrap()), "кислотная: {green:?}");
    // Обычный цвет кровли кодом — как есть.
    assert_eq!(tag_roof("#1a5947"), Some([26, 89, 71]));
    // Именной зелёный — тусклая крашеная жесть, а не кислотный CSS.
    let plain = tag_roof("green").unwrap();
    let (mx, mn) = (*plain.iter().max().unwrap() as f32, *plain.iter().min().unwrap() as f32);
    assert!(mx - mn <= 0.3 * (mx + mn).min(510.0 - mx - mn) + 1.0 && luma(plain) >= 70.0, "кислотный зелёный: {plain:?}");
    // Золото куполов — как прежде, яркое.
    assert_eq!(tag_roof("gold"), colour("gold").map(roof_rgb));
    // «Тёмно-серая» темнее серой, как её понимают в OSM.
    assert!(luma(colour("darkgrey").unwrap()) < luma(colour("grey").unwrap()));
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
fn cliff_foot_gets_scree_on_the_low_side() {
    let (m, _) = import(CLIFF, Options::default()).unwrap();
    let stones: Vec<&crate::map::Prop> = m.props.iter().filter(|q| q.asset.contains("boulder")).collect();
    assert!(stones.len() >= 20, "осыпи мало: {}", stones.len());
    // Кромка на y ≈ 13, низ на юге: камни под стенкой, не на ней, не наверху и
    // не дальше пары клеток от подножия.
    let r = m.relief.as_ref().unwrap();
    for q in &stones {
        assert!(q.pos[1] > 13.5 && q.pos[1] < 17.0, "камень не у подножия: {:?}", q.pos);
        assert!(r.steepest(q.pos, 0.2) < crate::relief::CLIFF, "камень на стенке: {:?}", q.pos);
    }
    // Крупные у стенки и мелочь.
    assert!(stones.iter().any(|q| q.scale > 0.6) && stones.iter().any(|q| q.scale < 0.4));
    // Через дорогу у подножия осыпь не ложится.
    let road = CLIFF.replace(
        "</osm>",
        r#" <node id="20" lat="51.53019" lon="-0.12495"/><node id="21" lat="51.53019" lon="-0.12415"/>
 <way id="30"><nd ref="20"/><nd ref="21"/><tag k="highway" v="residential"/></way>
</osm>"#,
    );
    let (m, _) = import(&road, Options::default()).unwrap();
    let path = m.paths.iter().find(|p| p.width > 0.5).expect("дороги нет");
    for q in m.props.iter().filter(|q| q.asset.contains("boulder")) {
        assert!(path.distance(q.pos) > path.width / 2.0, "камень на дороге: {:?}", q.pos);
    }
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
