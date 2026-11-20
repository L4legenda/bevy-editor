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
| `src/editor/model_paint_data.rs`, `model_paint_engine.rs`, `model_paint_gpu*.wgsl` | Material Painter brush coverage, stroke-start snapshots and straight-alpha paint/erase/compositing, adapted to Bevy | Material Painter contributors, local checkout `1456397558798e5ef51588270db85bf3c310332f`, [source](https://github.com/alla2001/material-painter) | GPL-3.0-or-later; notice in `vendor/material-painter/LICENSE` |
| `assets/font` | Liberation Sans, regular, bold, italic and bold italic | Google Corporation and Red Hat, Inc. | SIL Open Font License 1.1; included `OFL.txt` |
| `src/runtime/fbx.rs` | FBX mesh parsing through the ufbx Rust bindings | [ufbx and contributors](https://github.com/ufbx/ufbx-rust) | MIT OR Unlicense, see the `ufbx` crate |
| `native/uv` | Built-in PartUV Geometric CPU worker; Rust mesh preparation / Ward tree and C++ ABF pipeline | Zhaoning Wang and PartUV contributors; [PartUV](https://github.com/EricWang12/PartUV), source fork pinned in `vendor/dependencies.json` | Native worker GPL-3.0-or-later because it includes CGAL GPL components; PartUV project Apache-2.0 and original composite notice retained. See `native/uv/NOTICE.md` and `LICENSE`. PartField and CUDA are not built. |
| `native/uv/vendor/dawand` | DA Wand inference weights | Threedle, University of Chicago. [DAWand-Blender](https://github.com/threedle/DAWand-Blender/tree/7d0ee5b54e4ec40d6624c91c2a155e585cd05e92) | MIT, `native/uv/vendor/dawand/LICENSE` |
| `native/uv/vendor/xatlas.cpp` | xatlas chart generation and atlas packing | Jonathan Young and contributors; [xatlas](https://github.com/jpcy/xatlas) | MIT; `native/uv/vendor/LICENSE-xatlas` |
| `native/uv/vendor/headers.zip` | Eigen 3.4.0, libigl 2.5.0, CGAL 6.0.1, Boost 1.87.0, OpenABF and disabled easy_profiler headers | Respective authors; revisions and archive SHA-256 in `native/uv/vendor/dependencies.json` | MPL-2.0, GPL/LGPL by CGAL module, BSL-1.0, Apache-2.0 and MIT; full notices in the archive and `native/uv/licenses` |
| `native/uv/vendor/include` | nlohmann/json 3.11.3, stb_image_write | Niels Lohmann, Sean Barrett and contributors | MIT / public domain alternatives as identified in the preserved header notices |
| Embedded native UV runtime (Windows) | Visual C++ OpenMP runtime, `vcomp140.dll` | Microsoft; bundled from the MSVC redistributable tools | Microsoft Visual Studio redistributable runtime; no runtime installer is invoked |
