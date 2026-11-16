# Third-party components

| Path | Component | Author / source | License |
| --- | --- | --- | --- |
| `crates/hunyuan-native` | Rust port of the Hunyuan3D-2mini inference architecture | Tencent Hunyuan3D-2 contributors; https://github.com/Tencent-Hunyuan/Hunyuan3D-2 | Tencent Hunyuan 3D 2.0 Community License; `LICENSE-TENCENT` and `NOTICE` in the crate. Model weights are distributed separately. |
| `vendor/photocraft/crates/{geom,cms,color,raster,doc,paint,format}` | PhotoCraft core 0.23.1 | ArtCraft Team and the PhotoCraft contributors, local PhotoCraft checkout `e052eabf4be74612725400955d184f85d208c0f0` | MIT OR Apache-2.0; licenses in `vendor/photocraft` |
| `vendor/photocraft/crates/pcraft-format` | Shared PhotoCraft native format envelope | craft-libs contributors, `storytold/craft-libs` at `b8f7d93b00dfb8f172938f3d2d796ae62dcfe123` | MIT OR Apache-2.0; licenses in `vendor/photocraft` |
| `vendor/photocraft/crates/{engine,compose,ops,vector,text,algo,io,codecs,raw,affinity,plugins,ipp}` | PhotoCraft CPU layer engine and dependencies 0.23.1 | PhotoCraft contributors, local checkout `85d713723dcab81ff03db6db4b3258054a36c46e` | MIT OR Apache-2.0; licenses in `vendor/photocraft` |
| `vendor/photocraft/assets/fonts` | Inter, JetBrains Mono | Font authors named in accompanying OFL files | SIL Open Font License 1.1 |
| `vendor/photocraft/assets/dict` | SCOWL English dictionary | SCOWL contributors | Included `LICENSE-SCOWL.txt` |
| `assets/icons/lucide` | Lucide icons | Lucide Icons and Contributors | ISC; included `LICENSE` |
| `vendor/parley` | Parley 0.11.1, local ranged editing extension | The Parley Authors / Linebender | MIT OR Apache-2.0; included licenses |
| `src/editor/model_paint_data.rs`, `model_paint_engine.rs` | Material Painter brush coverage and straight-alpha stroke accumulation, adapted to Bevy | Material Painter contributors, local checkout `1456397558798e5ef51588270db85bf3c310332f`, [source](https://github.com/alla2001/material-painter) | GPL-3.0-or-later; notice in `vendor/material-painter/LICENSE` |
| `assets/font` | Liberation Sans, regular, bold, italic and bold italic | Google Corporation and Red Hat, Inc. | SIL Open Font License 1.1; included `OFL.txt` |
| Optional local UV runtime | PartUV 0.1.6, Geometric/ABF CPU path | Zhaoning Wang, Daniel Boxer; [PartUV](https://github.com/EricWang12/PartUV), Windows wheel from [UVgami](https://github.com/danielboxer/UVgami/releases/tag/partuv-v0.1.6) | PartUV project Apache-2.0 and bundled dependency notices. PartField AI segmentation and checkpoints are not used. |
| Optional local UV runtime | xatlas-python 0.0.9 / xatlas, atlas packing only | Michael Worchel, Jonathan Young; [xatlas-python](https://github.com/mworchel/xatlas-python), [xatlas](https://github.com/jpcy/xatlas) | MIT; notices included in the installed packages |
| Optional UV installer bootstrap | uv 0.9.11 | Astral contributors; [uv](https://github.com/astral-sh/uv) | MIT OR Apache-2.0; downloaded from the official release only when uv is unavailable |
