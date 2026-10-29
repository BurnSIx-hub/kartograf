# Запросы к Gemini для текстур

Gemini на «seamless tile» складывает 2×2 копии или светлеет к углу, поэтому
просим **обычную** картинку, а бесшовной делаем сами:

- природа — `seamless.py вход.jpg выход.png`;
- кладка, плиты — `stones.py вход.jpg выход.png`;
- доски, кирпич рядами — `planks.py вход.jpg выход.png [зерно] [сжатие поперёк]`.

После: проверить швы (на стыке около 1), сжать в WebP 88 в `assets/builtin`,
добавить строку в `library::BUILTIN_TEXTURES`.

## Общая часть (дописывать после предмета)

```
Hand-painted digital art style for a fantasy tabletop battle map: soft painterly brushwork, clean readable shapes. Flat even ambient lighting, same brightness everywhere, no vignette, no gradient, no focal point, no horizon, no perspective. Low contrast, details evenly distributed over the whole image. No border, no text, not a photograph.
```

Начало каждого запроса: `Draw a flat top-down view straight down onto …`, конец — `Square picture.`

## Природа → seamless.py

| Имя | Предмет | Запрет |
|---|---|---|
| gravel | loose grey-brown gravel: many small rounded pebbles of similar size, densely packed | No large rocks, plants or objects. |
| ice | the flat surface of thick frozen lake ice, pale blue and white, subtle internal cracks and tiny trapped air bubbles, light frost patches | No snow piles or objects. |
| sand | fine pale yellow sand with soft small wind ripples | No shells, rocks or objects. |
| mud | wet dark brown mud with a slight glossy sheen and soft shallow ruts | No footprints, puddles or objects. |
| swamp | murky green-brown swamp water with a thin film of algae and small floating duckweed | No plants sticking out, logs or objects. |
| forest_floor | forest floor covered with fallen brown and orange leaves, pine needles, small twigs and bits of moss | No mushrooms, stones or objects. |
| moss | a thick soft carpet of green moss with small natural variations | No flowers, stones or objects. |
| rocky_ground | rough rocky ground: dry grey-brown earth with many small stones and pebbles | No boulders, plants or objects. |
| ash | scorched ground covered with grey ash and small black charcoal bits | No fire, flames or objects. |
| lava | a cooling lava field: dark cracked basalt crust plates with thin glowing orange seams between them | No rocks sticking out or objects. |

## Подземелья → stones.py / planks.py

Начало: `Draw a flat top-down orthographic view of …`, дальше
стиль и свет из общей части, `No moss, puddles or objects` (кроме мха).

| Имя | Предмет | Сборка |
|---|---|---|
| dungeon_flagstones | a dungeon floor of worn dark grey flagstones: irregular rectangular stones of different sizes, about 8 stones across the width, thin dark mortar lines, subtle chips and wear | stones.py |
| dungeon_flagstones_moss | в том же чате: Same image, same stones and layout, only add green moss growing in the mortar lines and on some stone edges. | stones.py |
| cobblestone | a street of small rounded grey cobblestones, about 16 stones across the width, tightly packed in irregular rows, dark gaps | stones.py |
| brick_floor | a cellar floor of red-brown bricks laid in straight horizontal rows, running bond, about 12 brick rows from top to bottom, thin dark mortar lines | planks.py (ряды — по тёмным швам) |
| sandstone | a floor of large pale sandstone slabs, rectangular, varied sizes, about 6 across the width, thin dark joints | stones.py |
| marble | polished white and grey marble floor tiles, square, about 6 across the width, thin dark joints, subtle veins | stones.py |

## Замечания

- Отказ «нет доступа к этому контенту» (2026-10-28, проверено по частям): его
  вызывают «Generate an image. Square 1:1.» в начале и «The image shows an area of
  about 40 by 40 feet». Ни расширение, ни длина, ни имя набора ни при чём. Квадрат
  просить в самом конце: «Square picture.», без него выходит 1408×768. Имён чужих
  наборов всё равно не писать. Готовые тексты — `texture-prompts-ready.md`.
- Gemini часто сам кладёт 2×2 копии (снег, вода) — повтор каждые 2 клетки,
  проверять `tilecheck`.
- «12 камней в ширину» он понимает как ровную сетку квадратов.
