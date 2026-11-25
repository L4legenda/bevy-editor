# Material Painter integration

Source inspected: `G:\material-painter`, Material Painter 0.1.7. Its layout is
defined in `src/app/dock.rs`; feature boundaries are documented in its AGENTS.md.
The source repository is GPL-3.0-or-later. Brush coverage and straight-alpha
paint/erase math are adapted from its stamp/apply shaders; attribution and the
license are in `vendor/material-painter`. The workspace uses our Bevy UI and icons.

## First stage: Material Paint workspace

- Model mode selector: Object, Edit, UV, Paint (Рисование).
- The existing isolated 3D view, camera navigation, lighting and environment.
- A neighbouring 2D UV preview of the selected object's material slot; raster
  preparation runs on a worker, without blocking UI input.
- Left tool rail: brush, eraser, polygon fill, picker, path; brush configuration.
- Bottom resource shelf and category navigation; six procedural material swatches.
- Right tabs: layer stack, mesh maps, texture sets, preview; properties underneath.
- In-memory layer configuration, add/copy/delete/reorder/visibility and undo/redo.
- Per-model workspace state and a return to the original model-editing layout.

## Surface painting and editable GLB layers

- Brush strokes paint the visible model surface using its UV0 coordinates. Screen
  interpolation avoids connecting distant UV islands; BVH occlusion prevents
  painting through other surfaces. Alt-orbit and camera navigation retain priority.
- The texture pane also supports painting, erasing, polygon fill and a color picker.
- The Brushes shelf includes all 24 Material Painter stamp tips, rendered from
  the source library at 256 × 256, plus the editor's round brush. The GPU uses
  their grayscale coverage in both viewports. Tips can have a fixed angle or
  follow the direction of the stroke. Imported images use alpha when present
  and luminance for opaque images, with their aspect ratio preserved. Imported
  tips are available for the current editor session.
- Model painting and the image editor use the same ordered pointer samples,
  including Windows Ink and WinTab. Size and flow can respond to pressure
  independently. The GPU receives pressure per stamp. The stabilizer keeps a
  configurable distance between pointer and brush, and the stroke retains its
  spacing and uses one undo transaction per contact.
- The Textures shelf provides 25 tileable brush grain patterns. Strength caps
  coverage within a stroke. Repetition, rotation and inversion affect the pattern
  on the model surface or in UV coordinates. A GPU surface pass prepares the
  grain map once per texture set and brush configuration.
- Screen-space 3D brush and eraser strokes support X, Y, and Z symmetry about
  the object's local bounds center. A reflected texel checks the visible
  source surface against scene depth and object IDs. The two contributions use
  maximum coverage so they do not build up twice at the plane of symmetry.
- The 3D brush supports screen, surface, and UV projections. Surface stamps
  use a tangent frame at the hit point. UV stamps use the hit triangle's UV
  coordinates and world-to-UV area ratio. The worker builds a cached
  acceleration structure for projected strokes. Only stamp hits are resolved
  on the CPU. Coverage, grain, blending and the live preview remain on the
  GPU. Both modes support reflected stamps. Screen projection also has a
  paint-through option, and screen and surface stamps use an angle limit.
- Brush and eraser support per-stamp scatter, size jitter, and rotation jitter.
  Shift-click connects the previous endpoint to the cursor in the same
  viewport and texture set. It reaches both endpoints with the regular stamp
  spacing and records one undo entry.
- The Path tool edits points in model space or UV, previews the curve and
  up to 600 brush imprints, and supports linear and smoothed segments, closing the
  curve, and aligning the tip. Applying the path writes one GPU stroke.
  Undo restores the layer while retaining the editable points.
- Each material's texture set has its own resolution. The Sets panel offers
  512, 1024, 2048, and 4096. Resizing rebuilds that set's base, layer channels
  and raw masks on a worker and clears that set's undo and redo history.
  The saved GLB retains each set's resolution in its existing paint metadata.
- GLB primitives with complete UV islands distributed across UDIM tiles become
  separate texture sets per material and tile. UVs crossing a tile boundary
  within a face retain the ordinary repeating layout. Painting uses the
  selected tile's coordinates. The saved GLB includes tile-local UVs for its
  material maps and metadata that restores the global UV layout in the editor.
  The UV workspace frames the occupied tiles, and its native pack, unwrap,
  and preview run separately for each texture set. Results return to the
  original tiles and commit together in one undoable operation.
- Texture export writes eight PNG maps at each set's resolution. The toolbar
  exports all sets and the Sets tab can export the selected set. The folder
  picker chooses a parent directory for a fresh export folder. UDIM file names
  end in the tile number before the extension, for example
  `Object_Material_BaseColor.1013.png`. The export includes color with alpha,
  roughness, metallic, height, opacity, emission, tangent-space normals with
  height detail, and specular level.
- The Maps tab bakes the selected set in a background CPU job. UV rasterization
  uses tile-local coordinates and two texels of padding. The scene BVH is used
  for ambient occlusion, cavity, thickness, sky exposure, and bent normals.
  The same bake includes world normals, position, part ID, part random, and
  part height gradient. Maps can be previewed in UV and exported with the
  material. Results are cached for the current session and cancel if their
  source geometry changes.
- The low-poly bake also includes signed curvature, convex edge distance, and
  concave crease distance. Distance is measured in 3D along the current
  part's geometry. Coplanar triangulation and UV seams do not create
  features. The curvature map uses changes in shading normals and includes
  narrow bands at hard folds.
- The Sets panel can hide or reveal a set in the paint workspace. The viewport,
  brush, picker, fill, and path tools use the same visibility. Hovering over a
  set highlights it, selecting it plays a brief pulse, and the lamp pins the
  highlight. These controls apply to the current editing session.
- Polygon fill selects a triangle, polygon, quad strip, angle-limited surface,
  connected part, object, UV island, or material region in 3D and UV. It uses
  the active texture set and writes to the current channel or mask on the GPU.
  Shift-drag selects with a rectangle or lasso and commits on release. The
  optional through-selection mode includes hidden 3D triangles. Hover shows
  the boundary in the active viewport, including the silhouette of a closed
  3D selection. The preview draws up to 768 edges per view.
  Polygon and strip modes recognize quad pairs on triangulated meshes.
  An erase toggle and temporary Ctrl override use the same point and area
  selection. UV padding ranges from 0 to 16 texels and defaults to 2. The
  padding pass checks UV occupancy before painting to protect other charts.
- A stroke accumulates flow coverage with smooth hardness falloff. Stroke opacity
  caps the result, and a complete stroke creates one undo entry.
- Every object/material slot has an independent texture set and layer history.
  Paint/fill layers support opacity, visibility, ordering, duplication and deletion.
  Material presets set the fill color, roughness and metallic values.
- The layer panel supports Material Painter's eight blend modes in the GPU
  preview and in saved textures. Earlier GLBs default to Normal. Tangent-space
  normals keep their existing normal composition.
- Layers can be grouped up to eight levels deep. Moving, copying, and deleting
  a group includes its children. An invisible group hides its children and
  prevents painting inside it. A translucent group fades the combined result
  over the layers below. The GPU preview and GLB use the same composition order.
- Paint layers, fills, and groups can have an editable white or black mask.
  The brush can add black, gray, or white values to the mask in either viewport.
  GPU updates show the masked material while drawing, and a single asynchronous
  readback commits each finished stroke. The mask is saved losslessly in GLB
  separately from the layer channels and applies to all of them.
- The UV pane can show the resulting mask in grayscale. Uniform fills form an
  ordered mask stack above the painted pixels. Each fill has its own visibility,
  opacity, gray value, and one of the six Material Painter mask blend modes.
  All seven filters run after the fills in stack order: Levels, Invert, Contrast,
  Blur, Sharpen, Warp, and Noise. Mask settings stay editable after saving, and
  changes can be undone and redone.
- The CPU compositor and GPU brush share 256-entry lookup tables for consecutive
  fills and tonal filters. Spatial passes use separable Gaussian blur, unsharp
  masking, bilinear noise displacement and seeded fractal noise. The GPU limits
  filter work to the bounds of the touched UV tiles plus a cumulative sampling
  margin. It reuses two scratch buffers across all intermediate passes.
  The asynchronous readback retains the original painted values. The port
  rounds intermediate passes to the editor's 8-bit mask representation.
- Clouds and Height Gradient can drive the base mask or an ordinary mask layer.
  They use positions sampled from the active material slot's UV triangles and
  the object's local bounds. Generator settings and paint overrides are saved
  separately. On a generated base, the brush overpaints the result and the
  eraser restores its contribution. Generator layers blend below the filters.
  Reusable generator maps are prepared on the worker and uploaded when the GPU
  compositing context changes. Live stamps then use those maps without
  recalculating the generator.
- Texture channels are color, roughness, metallic, height (converted to normal
  detail), opacity, emission, normals and specular. Specular updates per texel
  on the GPU during a stroke. The paint stack keeps Material Painter's full
  specular range. The flattened `KHR_materials_specular` texture uses glTF's
  allowed strength range at the source index of refraction and preserves
  specular tint. The paint workspace projects imported UV1 and transformed
  textures into UV0 on a worker, respecting glTF texture coordinate overrides
  and sampler addressing. Linear RGBA8 maps keep their raw alpha as a source
  for specular strength.
- Live brush rasterization and compositing run on Bevy's GPU device. Geometry
  preparation and CPU document snapshots use the compute pool. Scene asset
  materials remain isolated from the model's working material.
- Save / Ctrl+S uses the model's existing atomic GLB save path. Standard glTF
  textures contain the flattened result, while versioned primitive extras point
  to losslessly compressed base textures and layer buffers in the same GLB.
  Reopening restores editable layers. Rename/duplicate naturally keep all data.
  Saving during a stroke waits for its worker results.
- Texture sets default to 1024 × 1024. Painting uses UV0. The UI links to the
  UV workspace for meshes without an unwrap. Overlapping UVs share texture
  pixels, as in ordinary UV painting.

The existing environment, scene tools and mesh-editing operations stay independent.
Mesh-map baking, mesh-map generator presets and input mixing, graph editing, SPP import, UDIMs and
upstream's separate path tracer are subsequent migration work.

## Interactive painting performance

Upstream `engine.rs::add_region` and the stamp/composite passes keep textures on
the GPU, scissor draws to the stroke region, and resume compositing at the changed
layer. The first Bevy port instead rasterized each stamp independently with a BVH
ray per texel, recomposited eight complete maps and replaced nine Image assets on
each result. At 1024px that uploaded 36 MiB per batch and repeatedly prepared GPU
textures/bind groups. Rebuilding layer panels and copying/comparing pixel buffers
on the main thread added stalls.

The live path now uses resident GPU buffers and the application's existing render
device/queue, with no second graphics context:

1. Prepare transformed triangle vertices on a worker. Render depth and object/face
   IDs for the stroke view; the UV pass supports ordinary and overlapping UVs.
2. Rasterize screen/UV brush coverage into a persistent floating-point mask. Flow
   accumulates there. Only stamp coordinates and brush uniforms cross from CPU to
   GPU while drawing; a stationary brush enqueues no extra work.
3. Above 1024px, fragments flag touched 8 × 8 UV tiles. A GPU compaction pass builds
   an indirect dispatch list; apply/composite/present visit those tiles only.
   Height painting includes neighboring tiles for derived normal gradients.
   At 1024px and below, simple dense dispatches avoid tile-list/indirect overhead.
   Both paths use a stroke-start GPU snapshot for opacity caps and straight-alpha
   erasing. The lower layer stack is cached until the document/layer configuration
   changes; compositing resumes at the active layer, including visible layers above.
4. GPU passes pack color/opacity and metallic/roughness, derive height/normal detail,
   and copy into the existing material/2D preview textures without recreating them.
5. At release, asynchronously copy the active layer to a readback buffer. A worker
   checks changes and publishes one undo transaction into the existing CPU/GLB
   document. GPU submission order and stroke IDs preserve consecutive strokes.
   Save/close wait for outstanding snapshots; runtime code never waits on the GPU.

Shader pipelines are created at renderer startup, before the first brush press.
Depth/visibility checks run only after a texel passes the brush-disc test.
The first display pass initializes the full persistent output texture; later
partial passes preserve untouched pixels. The final copy into Bevy's existing
sRGB-compatible material textures is still a full GPU-to-GPU copy. Geometry is
still rasterized for each stamp batch to preserve overlapping UV projections.
Initial previews and standalone layer configuration/undo still use the CPU
compositor; they are not on the interactive stamp path. The old CPU rasterizers
remain under `cfg(test)` as correctness/performance references.

Native GPU tests cover live color before any CPU readback, all eight channels,
occlusion, erasing/fill, orthographic views, stable image allocations, history and
GLB save/reopen. The mask test checks the UV pixels before and during a held
stroke through a Multiply fill, Invert, and all four spatial filters. It also
checks undo and redo, overpainting and erasing a generated base, and GLB
restoration of the raw mask, generator and filter settings.
The brush tests check the shelf selection, PNG alpha import with aspect ratio
preservation, and a ring stamp with a transparent center in both viewports.
Synthetic pen contacts verify pressure-dependent coverage and diameter on the
GPU, including a press and release in one frame and a single undo per contact.
The viewport test checks an X-reflected stroke on the visible surface and on
the occluded geometry behind it. It also samples the dark and light cells of a
grain pattern in both UV and surface coordinates.
It also checks the difference between surface and UV footprints on a stretched
UV chart, symmetry in both projections, and screen painting through an
occluding surface.
Polygon fill tests check UV seams, surface-angle and quad-strip traversal, UV
triangle picking, and GPU coverage for a connected part and a whole object with
overlapping geometry.
The GPU input scenario checks selection with a 3D box and UV lasso, occlusion,
hover outlines, erase fill, UV gap padding and undo. The topology test also
checks quad reconstruction in a triangulated copy.
The GPU input scenario also checks straight strokes in both views, a single
undo for the connecting stroke, and the reduced footprint with size jitter.
It verifies dragging a UV path point before application, retaining the path
after undo, and applying points placed on the 3D model.
The texture set scenario paints after resizing, switches between a 512 and a
1024 set, and reads both sizes back from the exported GLB. It also measures a
pinned highlight in a captured frame and paints through a hidden foreground
set to the visible surface behind it. The same scenario exports both UDIM
sets as PNG, checks image dimensions and painted pixels, and exports one tile
again into a new folder.
`gpu_model_paint_benchmark_40k` measures a held stroke over 40,000
triangles and asserts zero readbacks during drawing; the earlier CPU benchmark
remains available for comparison.

`tools/paint-perf/README.md` describes the shared 262,144-triangle comparison with
upstream, including measurement boundaries and reproducible commands.

## Initial migration outline

The [port plan](material-paint-port-plan.md) tracks the current implementation and verification.

| Feature | Source modules | Integration boundary |
|---|---|---|
| Generator presets, generator input mixing, compressed stroke history | `engine.rs`, `engine/`, `textures.rs`, `undo.rs`, `shaders/` | Add mesh-map inputs and pack stroke snapshots after GPU readback |
| Moving UDIM islands between sets and additional mesh formats | `app/texture_sets.rs`, `mesh.rs`, `import.rs`, `unwrap.rs` | Update set assignment on cross-tile edits |
| Additional mesh maps, high-poly/cage baking, GPU jobs and persistent cache | `bake.rs`, `bake_cache.rs`, `cage.rs`, `app/cage_paint.rs` | Add missing map passes, projection, user settings, and disk reuse |
| Smart materials, masks, generators, shelf and online libraries | `presets.rs`, `library.rs`, `app/library_ui.rs` | Asset catalog, thumbnails and downloads |
| Material Designer and custom graph nodes | `graph/`, `app/graph_ui.rs` | Native graph workspace, typed connections and GPU evaluation |
| Shader Maker and preview shaders | `app/shader_ui.rs`, `app/preview_shader.rs`, `graph/shading.rs` | Shader editor, diagnostics and preview pipelines |
| Substance import, export presets and channel packing | `app/project_io.rs`, `spp_import.rs`, `app/spp_*`, `export.rs` | Extend the current embedded GLB layer document |
| Ray-traced preview, post effects, pen and SpaceMouse | `raytrace.rs`, `post.rs`, `pen.rs`, `spacemouse.rs` | Optional viewport backends and native input |

The source uses eframe/egui 0.37 and wgpu 30. Its UI and renderer cannot be
installed unchanged as a Bevy widget. Reuse of its engine must preserve its
license notices and replace the egui/window/GPU integration points; replacing
the application's existing environment system is not part of the requested port.

### Проекция high-poly

В панели «Карты» выберите файл high-poly GLB, OBJ или FBX и нажмите «Запечь выбранный набор». Обе модели должны совпадать в пространстве. OBJ использует координаты файла, FBX учитывает трансформации узлов и переводит единицы в метры. При сопоставлении имён `Plate_low` и `Plate_high.001` считаются одной частью. Пределы лучей задаются отдельно снаружи и изнутри в процентах от диагонали видимой модели.

UV-предпросмотр и экспорт включают `BakedNormal`, `BakedHeight` и `ProjectionCoverage`. Нормали записываются в касательном пространстве, нейтральная высота соответствует серому, белые участки карты попаданий показывают прямые попадания лучей. Настройка «Заполнять промахи» берёт ближайшую точку high-poly в пределах четырёх максимальных дальностей луча. Эту точку также используют карты рельефа, видимости, толщины, нормалей и связных частей. Плавная кривизна интерполируется по вершинам, резкие сгибы и контуры сильной кривизны определяют расстояния внутри одной связной части. При промахе без заполнения карты получают нейтральные значения.

Сопоставление по именам ограничивает выбор поверхности при проекции. Другие детали high-poly участвуют в окклюзии, оценке толщины и открытости небу. Карта положения хранит координаты точки low-poly под UV-текселем. Настройки high-poly действуют в сеансе, а повторное запекание перечитывает файл.

Под кнопкой запекания можно выбрать «Черновик», «Стандарт», «Высокое» или «Максимум». Пресеты используют соответственно 16/8/8, 64/32/32, 160/64/64 и 384/128/128 лучей для AO, толщины и неба. Для проекции high-poly они усредняют нормали и высоту по сеткам 1×1, 2×2, 3×3 и 8×8 внутри UV-треугольника. По умолчанию редактор выбирает «Черновик» для фонового запекания на CPU.

Перед расширением UV-краёв редактор автоматически сглаживает шум на картах AO, впадин, толщины, открытости небу и нормали видимости. Пресеты выполняют соответственно четыре, три, два и один проход. Фильтр берёт соседние тексели того же UV-острова и связной части с учётом положения и нормали поверхности.

Повторное запекание загружает сжатые карты из дискового кэша, если геометрия, UV, файлы high-poly и оболочки, разрешение и параметры совпадают. Проверяется содержимое файлов. Обновлённый источник или повреждённая запись запускают обычное запекание. На Windows записи хранятся в `%LOCALAPPDATA%\BevyEditor\bake-cache`. При превышении 2 ГиБ старые записи удаляются.

Запечённый набор питает одиннадцать пресетов генераторов масок. Среди них износ рёбер, загрязнения, пыль, подтёки, маски тонких участков и вариации по деталям. Пресет можно выбрать для основы или слоя маски. Снимок карт сохраняется в GLB и обновляется после запекания этого набора. Порог, мягкость, масштаб, шум, зерно и инверсия остаются редактируемыми.

Для «Маски по ID» выберите цвет детали из палитры запечённого набора. Увеличьте допуск, чтобы включить близкие цвета. Если в старом GLB сохранены только скалярные карты генераторов, повторное запекание добавит карту ID. Выбранный цвет и допуск сохраняются вместе со слоем.

В разделе «Входы генератора» можно смешивать 20 встроенных источников, включая кривизну, окклюзию, высоту, ID, ячейки, волокна дерева и царапины. Положительный вес добавляет вклад, отрицательный вычитает его. Нажатие на значение меняет знак веса, а кнопка «Веса пресета» восстанавливает исходные значения. Для рёбер и складок можно менять ширину. Настройки записываются в GLB.

В разделе «Пользовательские карты» доступны направленная открытость, ветровые отложения, контакт деталей и расстояние от точки. Добавьте карту, задайте параметры и запеките набор. Карту можно просмотреть в UV, экспортировать как PNG или добавить во «Входы генератора». Один генератор смешивает до четырёх пользовательских карт. Размеры и координаты точки задаются в пространстве модели, приведённой к двум единицам по наибольшей стороне.

Карта «Потёки по поверхности» накапливает жидкость, попадающую на модель выше по течению. При запекании учитываются укрытия, сгибы и открытые края. Настройте направление, разброс, дальность, силу и извилистость потёков. Как и другие пользовательские карты, потёки сохраняются через GLB и могут управлять маской.

Кнопка «Выбрать оболочку…» загружает cage из GLB, OBJ или FBX. Подходит копия low-poly с перемещёнными вершинами при сохранении порядка треугольников и связности. В файле с несколькими объектами имя `Plate_cage` сопоставляется с `Plate_low`. Луч стартует на оболочке, идёт к поверхности low-poly и дальше на заданную внутреннюю дальность. Кнопка «Убрать оболочку» возвращает настройку внешней дальности.

«Подгонять оболочку» доступно при выбранном high-poly без отдельного файла cage. При запекании редактор подбирает внешний и внутренний пределы вдоль нормалей low-poly, сохраняет общую дальность у вершин на швах и учитывает соседние части high-poly. Базовые дальности задают исходный масштаб, а поиск ограничен восемью базовыми дальностями и диагональю видимой модели. Итоговый статус показывает число вершин, у которых оболочка расширена, сужена или для которых запас подобрать не удалось.

«Предпросмотр оболочки» показывает проверку лучей в 3D до запекания. Зелёный означает попадание, жёлтый — попадание у предела дальности, красный — промах, фиолетовый — попадание в другую часть. После смены настроек проверка пересчитывается в фоне. Проценты относятся к диагностическим пробам, а карта попаданий после запекания считается по UV-текселям.

«Кисть оболочки» доступна для оболочки, построенной в редакторе. ЛКМ увеличивает дальность под кистью, Ctrl+ЛКМ уменьшает, Shift+ЛКМ возвращает участок к исходной дальности. При включённой автоподгонке исходной служит подогнанная оболочка. Esc отменяет текущий мазок, а «Сбросить ручную правку» сбрасывает все мазки на текущем объекте. Изменения действуют в сеансе и учитываются во всех наборах этого объекта. Предпросмотр пересчитывает лучи в фоне с уже загруженной high-poly, сохраняя предыдущую оболочку на экране до обновления.
