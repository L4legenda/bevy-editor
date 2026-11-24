// Synthetic HDF5 and SPP fixtures adapted from SubCraft (MIT).
// Revision 5adf219b18b5b33bc26ca82bd972af601ab0ed21, see vendor/subcraft-spp/LICENSE-MIT.
#![allow(dead_code)]
use std::collections::BTreeMap;
// ----- a minimal HDF5 writer (superblock v0, symbol-table groups, contiguous byte datasets) ----------

#[derive(Default)]
pub struct Node {
    children: BTreeMap<String, Node>,
    data: Option<Vec<u8>>,
}

fn pad8(out: &mut Vec<u8>) {
    while out.len() % 8 != 0 {
        out.push(0);
    }
}

fn object_header(out: &mut Vec<u8>, messages: &[(u16, Vec<u8>)]) -> u64 {
    pad8(out);
    let addr = out.len() as u64;
    let body: usize = messages
        .iter()
        .map(|(_, d)| 8 + d.len().div_ceil(8) * 8)
        .sum();
    out.extend([1, 0]);
    out.extend((messages.len() as u16).to_le_bytes());
    out.extend(1u32.to_le_bytes());
    out.extend((body as u32).to_le_bytes());
    out.extend([0; 4]);
    for (kind, data) in messages {
        out.extend(kind.to_le_bytes());
        out.extend(((data.len().div_ceil(8) * 8) as u16).to_le_bytes());
        out.extend([0; 4]);
        out.extend(data);
        out.resize(out.len() + data.len().div_ceil(8) * 8 - data.len(), 0);
    }
    addr
}

/// Write a group (and everything under it); returns (object header, b-tree, heap) addresses.
fn write_group(out: &mut Vec<u8>, node: &Node) -> (u64, u64, u64) {
    let mut entries: Vec<(String, u64)> = Vec::new();
    for (name, child) in &node.children {
        let addr = match &child.data {
            Some(bytes) => {
                pad8(out);
                let data_addr = out.len() as u64;
                out.extend(bytes);
                let space = [
                    vec![1, 1, 0, 0, 0, 0, 0, 0],
                    (bytes.len() as u64).to_le_bytes().to_vec(),
                ]
                .concat();
                let ty = vec![0x10, 0, 0, 0, 1, 0, 0, 0, 0, 0, 8, 0];
                let layout = [
                    vec![3, 1],
                    data_addr.to_le_bytes().to_vec(),
                    (bytes.len() as u64).to_le_bytes().to_vec(),
                ]
                .concat();
                object_header(out, &[(1, space), (3, ty), (8, layout)])
            }
            None => write_group(out, child).0,
        };
        entries.push((name.clone(), addr));
    }
    // Local heap data: an empty name first, then the names.
    let mut heap_data = vec![0u8; 8];
    let mut offsets = Vec::new();
    for (name, _) in &entries {
        offsets.push(heap_data.len() as u64);
        heap_data.extend(name.as_bytes());
        heap_data.push(0);
        while heap_data.len() % 8 != 0 {
            heap_data.push(0);
        }
    }
    pad8(out);
    let heap_data_addr = out.len() as u64;
    out.extend(&heap_data);
    pad8(out);
    let heap = out.len() as u64;
    out.extend(b"HEAP");
    out.extend([0; 4]);
    out.extend((heap_data.len() as u64).to_le_bytes());
    out.extend(u64::MAX.to_le_bytes());
    out.extend(heap_data_addr.to_le_bytes());
    pad8(out);
    let snod = out.len() as u64;
    out.extend(b"SNOD");
    out.extend([1, 0]);
    out.extend((entries.len() as u16).to_le_bytes());
    for ((_, addr), off) in entries.iter().zip(&offsets) {
        out.extend(off.to_le_bytes());
        out.extend(addr.to_le_bytes());
        out.extend([0; 4 + 4 + 16]);
    }
    pad8(out);
    let btree = out.len() as u64;
    out.extend(b"TREE");
    out.extend([0, 0]);
    out.extend(1u16.to_le_bytes());
    out.extend(u64::MAX.to_le_bytes());
    out.extend(u64::MAX.to_le_bytes());
    out.extend(0u64.to_le_bytes());
    out.extend(snod.to_le_bytes());
    out.extend(0u64.to_le_bytes());
    let st = [btree.to_le_bytes(), heap.to_le_bytes()].concat();
    let header = object_header(out, &[(0x11, st)]);
    (header, btree, heap)
}

pub fn hdf5_file(files: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut root = Node::default();
    for (path, data) in files {
        let mut node = &mut root;
        for part in path.split('/') {
            node = node.children.entry(part.to_string()).or_default();
        }
        node.data = Some(data.clone());
    }
    let mut out = vec![0u8; 96];
    let (header, btree, heap) = write_group(&mut out, &root);
    out[..8].copy_from_slice(&[0x89, b'H', b'D', b'F', 0x0d, 0x0a, 0x1a, 0x0a]);
    out[13] = 8;
    out[14] = 8;
    let eof = out.len() as u64;
    out[40..48].copy_from_slice(&eof.to_le_bytes());
    out[64..72].copy_from_slice(&header.to_le_bytes());
    out[72..76].copy_from_slice(&1u32.to_le_bytes());
    out[80..88].copy_from_slice(&btree.to_le_bytes());
    out[88..96].copy_from_slice(&heap.to_le_bytes());
    out
}

// ----- Qt-style blobs ---------------------------------------------------------------------------------

pub fn qstring(s: &str) -> Vec<u8> {
    let units: Vec<u8> = s.encode_utf16().flat_map(|u| u.to_be_bytes()).collect();
    [(units.len() as u32).to_be_bytes().to_vec(), units].concat()
}

pub enum Q {
    Str(&'static str),
    UInt(u32),
    UInt64(u64),
}

pub fn properties(props: &[(&str, Q)]) -> Vec<u8> {
    let mut out = (props.len() as u32).to_be_bytes().to_vec();
    for (k, v) in props {
        out.extend(qstring(k));
        match v {
            Q::Str(s) => {
                out.extend(10u32.to_be_bytes());
                out.push(0);
                out.extend(qstring(s));
            }
            Q::UInt(n) => {
                out.extend(3u32.to_be_bytes());
                out.push(0);
                out.extend(n.to_be_bytes());
            }
            Q::UInt64(n) => {
                out.extend(5u32.to_be_bytes());
                out.push(0);
                out.extend(n.to_be_bytes());
            }
        }
    }
    out
}

fn le(out: &mut Vec<u8>, v: u32) {
    out.extend(v.to_le_bytes());
}
/// A `texture-cache` entry holding an RG8 buffer as a single LZ4 literal run.
pub fn cache_blob(width: u32, height: u32, pixels: &[u8]) -> Vec<u8> {
    let mut block = Vec::new();
    let n = pixels.len();
    if n < 15 {
        block.push((n as u8) << 4);
    } else {
        block.push(0xf0);
        let mut rest = n - 15;
        while rest >= 255 {
            block.push(255);
            rest -= 255;
        }
        block.push(rest as u8);
    }
    block.extend(pixels);
    let mut out = properties(&[
        ("bufferSize", Q::UInt(pixels.len() as u32)),
        ("compression", Q::Str("lz4")),
        ("height", Q::UInt(height)),
        ("internalFormat", Q::UInt(33323)),
        ("version", Q::Str("v1")),
        ("width", Q::UInt(width)),
    ]);
    out.extend((block.len() as u32).to_be_bytes());
    out.extend(block);
    out
}

pub fn bitmap_blob(pixels: [u8; 4]) -> Vec<u8> {
    bitmap_row(&pixels)
}

pub fn bitmap_row(pixels: &[u8]) -> Vec<u8> {
    let data = lz4_flex::block::compress(pixels);
    let mut blob = properties(&[
        ("compression", Q::Str("lz4")),
        ("width", Q::UInt(pixels.len() as u32 / 4)),
        ("height", Q::UInt(1)),
        ("pitch", Q::UInt(pixels.len() as u32)),
        ("dataSizeRaw", Q::UInt(pixels.len() as u32)),
        ("dataSizeLZ4", Q::UInt(data.len() as u32)),
        ("format", Q::Str("RGBA8")),
    ]);
    blob.extend(data);
    blob
}

pub struct DocWriter {
    out: Vec<u8>,
    classes: Vec<&'static str>,
}

impl DocWriter {
    fn u32(&mut self, v: u32) {
        self.out.extend(v.to_le_bytes());
    }
    fn string(&mut self, s: &str) {
        self.u32(s.len() as u32);
        self.out.extend(s.as_bytes());
    }
    /// Class word: the definition the first time, the index afterwards.
    fn class(&mut self, name: &'static str, props: &[(&str, u32)]) {
        if let Some(i) = self.classes.iter().position(|c| *c == name) {
            self.u32(i as u32);
        } else {
            self.u32(u32::MAX);
            self.string(name);
            self.u32(props.len() as u32);
            for (n, t) in props {
                self.string(n);
                self.u32(*t);
            }
            self.classes.push(name);
        }
    }
}

pub fn document(normal_syscoord: i32) -> Vec<u8> {
    document_with_bitmap(normal_syscoord, None)
}

pub fn document_v0() -> Vec<u8> {
    fn string(out: &mut Vec<u8>, value: &str) {
        out.extend((value.len() as u32).to_le_bytes());
        out.extend(value.as_bytes());
    }
    fn typed(kind: u8, value: impl AsRef<[u8]>) -> Vec<u8> {
        let mut out = vec![kind];
        out.extend(value.as_ref());
        out
    }
    fn text(value: &str) -> Vec<u8> {
        let mut out = vec![16];
        string(&mut out, value);
        out
    }
    fn object(class: &str, fields: &[(&str, Vec<u8>)]) -> Vec<u8> {
        let mut body = Vec::new();
        string(&mut body, class);
        body.extend((fields.len() as u16).to_le_bytes());
        for (name, value) in fields {
            string(&mut body, name);
            body.extend(value);
        }
        let mut out = vec![18, 0];
        out.extend((body.len() as u32).to_le_bytes());
        out.extend(body);
        out
    }
    fn list(kind: u8, items: &[Vec<u8>]) -> Vec<u8> {
        let mut out = vec![kind];
        let length = 4 + items.iter().map(Vec::len).sum::<usize>();
        out.extend((length as u32).to_le_bytes());
        out.extend((items.len() as u32).to_le_bytes());
        out.extend(items.iter().flatten());
        out
    }

    let uniform = object(
        "DataSourceUniform",
        &[
            ("channelTypes", typed(12, 1u64.to_le_bytes())),
            (
                "color",
                typed(3, [0.25f32, 0.5, 0.75].map(f32::to_le_bytes).concat()),
            ),
        ],
    );
    let fill = object("DataActionFill", &[("sources", list(19, &[uniform]))]);
    let actions = object("DataStackActions", &[("items", list(19, &[fill]))]);
    let layer = object(
        "DataLayerColor",
        &[
            ("label", text("Rust")),
            ("actions", actions),
            ("maskActions", vec![20, 255]),
        ],
    );
    let sub_stack = object("DataStackLayers", &[("items", list(19, &[layer]))]);
    let group = object(
        "DataLayerGroup",
        &[("label", text("Old group")), ("subStack", sub_stack)],
    );
    let channel = |bits, format: i64| {
        object(
            "DataChannel",
            &[
                ("type", typed(21, u64::to_le_bytes(bits))),
                ("format", typed(9, format.to_le_bytes())),
            ],
        )
    };
    let stack = object(
        "DataMaterialStack",
        &[
            ("channels", list(19, &[channel(1, 0), channel(128, 1)])),
            (
                "stack",
                object("DataStackLayers", &[("items", list(19, &[group]))]),
            ),
        ],
    );
    let material = object(
        "DataMaterial",
        &[
            ("sceneMaterialName", text("Cube")),
            ("sceneMaterialUID", typed(12, 4242u64.to_le_bytes())),
            (
                "resolutionLog2",
                typed(6, [8i32; 2].map(i32::to_le_bytes).concat()),
            ),
            ("additionalMaps", list(17, &[])),
            ("stacks", list(19, &[stack])),
        ],
    );
    let mut out = [0x1b7c_2fddu32, 0, 0].map(u32::to_le_bytes).concat();
    out.extend(object(
        "DataDocument",
        &[("materials", list(19, &[material]))],
    ));
    out
}

pub fn document_with_bitmap(normal_syscoord: i32, bitmap: Option<&str>) -> Vec<u8> {
    document_options(normal_syscoord, bitmap, TestMask::None)
}

pub fn document_with_polygon_mask() -> Vec<u8> {
    document_options(0, None, TestMask::Polygon)
}

pub fn document_with_strokes() -> Vec<u8> {
    document_options(0, None, TestMask::Strokes)
}

pub fn document_with_paint_fill() -> Vec<u8> {
    document_options(0, None, TestMask::PaintFill)
}

pub fn document_with_mask_effects(url: &str) -> Vec<u8> {
    document_options(0, None, TestMask::Effects(url, false))
}

pub fn document_with_mask_outline(url: &str) -> Vec<u8> {
    document_options(0, None, TestMask::Effects(url, true))
}

pub fn document_with_ordered_mask_filters() -> Vec<u8> {
    document_options(0, None, TestMask::OrderedFilters)
}

pub fn document_with_anchors() -> Vec<u8> {
    document_options(0, None, TestMask::Anchors)
}

pub fn document_with_udim() -> Vec<u8> {
    document_options(0, None, TestMask::Udim)
}

pub fn document_with_stencil(url: &str) -> Vec<u8> {
    document_options(0, None, TestMask::Stencil(url, false, None))
}

pub fn document_with_uv_stencil(url: &str) -> Vec<u8> {
    document_options(0, None, TestMask::Stencil(url, true, None))
}

pub fn document_with_procedural_stencil(uv: bool, inverted: bool) -> Vec<u8> {
    document_options(
        0,
        None,
        TestMask::Stencil(
            "resource://starter_assets/drop_splatter/drop_splatter?version=1.sbsar",
            uv,
            Some(inverted),
        ),
    )
}

pub fn document_with_bitmap_tip(url: &str) -> Vec<u8> {
    document_with_directed_tip(url, TipStroke::Plain)
}

#[derive(Clone, Copy, PartialEq)]
pub enum TipStroke {
    Plain,
    Rotated,
    Follow,
    FollowTap,
    Jittered,
    Splatter {
        uv: bool,
        inverted: bool,
        horizontal: bool,
    },
}

pub fn document_with_directed_tip(url: &str, stroke: TipStroke) -> Vec<u8> {
    document_options(0, None, TestMask::BitmapTip(url, stroke))
}

pub fn document_with_pressure() -> Vec<u8> {
    document_options(0, None, TestMask::Pressure)
}

enum TestMask<'a> {
    None,
    Polygon,
    Strokes,
    PaintFill,
    Effects(&'a str, bool),
    OrderedFilters,
    Anchors,
    Udim,
    Stencil(&'a str, bool, Option<bool>),
    BitmapTip(&'a str, TipStroke),
    Pressure,
}

fn document_options(normal_syscoord: i32, bitmap: Option<&str>, mask: TestMask<'_>) -> Vec<u8> {
    let strokes = matches!(mask, TestMask::Strokes);
    let paint_fill = matches!(mask, TestMask::PaintFill);
    let anchors = matches!(mask, TestMask::Anchors);
    let udim = matches!(mask, TestMask::Udim);
    let pressure = matches!(mask, TestMask::Pressure);
    let stencil = match mask {
        TestMask::Stencil(url, uv, inverted) => Some((url, uv, inverted)),
        _ => None,
    };
    let tip = match mask {
        TestMask::BitmapTip(url, stroke) => Some((url, stroke)),
        _ => None,
    };
    let mask_image = match mask {
        TestMask::Effects(url, _) => Some(url),
        _ => None,
    };
    let mut w = DocWriter {
        out: Vec::new(),
        classes: Vec::new(),
    };
    w.out.extend(0x1b7c_2fddu32.to_le_bytes());
    for v in [1, 0, 0, 0] {
        w.u32(v);
    }
    // DataDocument { materials: list of objects, normalMapSyscoord: int }
    w.u32(0x100);
    w.class(
        "DataDocument",
        &[("materials", 19), ("normalMapSyscoord", 9)],
    );
    w.u32(1);
    // DataMaterial { additionalMaps: struct list, resolutionLog2: ivec2, sceneMaterialName, sceneMaterialUID, stacks }
    w.u32(0x101);
    w.class(
        "DataMaterial",
        &[
            ("additionalMaps", 17),
            ("resolutionLog2", 6),
            ("sceneMaterialName", 16),
            ("sceneMaterialUID", 12),
            ("stacks", 19),
            ("useUVTiles", 10),
            ("uvTiles", 19),
        ],
    );
    w.u32(2);
    w.class("DataAdditionalMap", &[("urlToBitmapRes", 16), ("usage", 9)]);
    w.string("/Position Cube?version=bbbb");
    w.u32(4);
    w.string("/Ambient Occlusion Map from Mesh Cube?version=aaaa");
    w.u32(1);
    w.u32(8);
    w.u32(8);
    w.string("Cube");
    w.out.extend(4242u64.to_le_bytes());
    w.u32(1);
    // DataMaterialStack { channels: list, stack: object }
    w.u32(0x102);
    w.class("DataMaterialStack", &[("channels", 19), ("stack", 18)]);
    w.u32(2);
    for (bits, format) in [(1u64, 0u32), (128, 1)] {
        w.u32(0x200 + bits as u32);
        w.class(
            "DataChannel",
            &[("format", 9), ("type", 21), ("userName", 16)],
        );
        w.u32(format);
        w.out.extend(bits.to_le_bytes());
        w.string("");
    }
    // The layer stack: one fill layer with a uniform color source, one group holding a paint layer.
    w.u32(0x103);
    w.class("DataStackLayers", &[("items", 19)]);
    w.u32(2);
    w.u32(0x300);
    w.class(
        "DataLayerColor",
        &[
            ("actions", 18),
            ("enabled", 10),
            ("label", 16),
            ("maskActions", 18),
            ("perChannelBlending", 17),
            ("geometryMaskType", 9),
            ("enabledMeshDefault", 10),
            ("enabledMeshList", 19),
            ("enabledUVTileDefault", 10),
            ("enabledUVTileList", 19),
        ],
    );
    w.u32(0x301);
    w.class("DataStackActions", &[("items", 19)]);
    w.u32(if anchors { 3 } else { 1 });
    w.u32(0x302);
    w.class("DataActionFill", &[("channelTypes", 12), ("sources", 19)]);
    w.out.extend(u64::MAX.to_le_bytes());
    w.u32(if bitmap.is_some() { 2 } else { 1 });
    w.u32(0x303);
    w.class("DataSourceUniform", &[("channelTypes", 12), ("color", 3)]);
    w.out.extend(1u64.to_le_bytes());
    for c in [0.25f32, 0.5, 0.75] {
        w.out.extend(c.to_le_bytes());
    }
    if let Some(url) = bitmap {
        w.u32(0x304);
        w.class("DataSourceBitmap", &[("channelTypes", 12), ("bitmap", 18)]);
        w.out.extend(128u64.to_le_bytes());
        w.u32(0x305);
        w.class("DataBitmap", &[("urlToBitmapRes", 16)]);
        w.string(url);
    }
    if anchors {
        write_anchor(&mut w, 0x702, 16_777_217);
        write_uniform(&mut w, 0x704, 0.);
    }
    w.out.push(1);
    w.string("Rust");
    if matches!(mask, TestMask::Polygon) {
        write_polygon_mask(&mut w);
    } else if strokes {
        write_strokes(&mut w, 0x400, true, None, None, false, false);
    } else if let Some(url) = mask_image {
        write_mask_effects(&mut w, url, matches!(mask, TestMask::Effects(_, true)));
    } else if matches!(mask, TestMask::OrderedFilters) {
        w.u32(0x600);
        w.class("DataStackActions", &[("items", 19)]);
        w.u32(4);
        write_uniform(&mut w, 0x601, 0.25);
        w.u32(0x603);
        w.class("DataActionFilterProcedural", &[("procedural", 18)]);
        w.u32(0x604);
        w.class("DataProcedural", &[("urlToSbsRes", 16)]);
        w.string("starter/invert/invert");
        w.u32(0x605);
        w.class(
            "DataActionGroup",
            &[("perChannelBlending", 19), ("subStack", 18)],
        );
        w.u32(1);
        w.u32(0x606);
        w.class("DataBlending", &[("blendingMode", 9)]);
        w.u32(4); // Multiply
        w.u32(0x607);
        w.class("DataStackActions", &[("items", 19)]);
        w.u32(1);
        write_uniform(&mut w, 0x608, 0.5);
        w.u32(0x60a);
        w.class("DataActionFilterLevels", &[("levels", 18)]);
        w.u32(0x60b);
        w.class(
            "DataLevels",
            &[("gamma", 3), ("outputMinimum", 3), ("outputMaximum", 3)],
        );
        for value in [2.0_f32, 0.2, 0.6] {
            for _ in 0..3 {
                w.u32(value.to_bits());
            }
        }
    } else if anchors {
        w.u32(0x710);
        w.class("DataStackActions", &[("items", 19)]);
        w.u32(3);
        write_uniform(&mut w, 0x712, 0.25);
        write_anchor(&mut w, 0x714, 16_777_218);
        write_uniform(&mut w, 0x716, 0.75);
    } else {
        w.u32(0);
    }
    w.u32(0); // no per-channel blending
    w.u32(u32::from(mask_image.is_some()));
    w.out.push(1);
    w.u32(u32::from(mask_image.is_some()));
    if mask_image.is_some() {
        w.u32(0x605);
        w.class("DataString", &[("value", 16)]);
        w.string("Right");
    }
    w.out.push(1);
    w.u32(u32::from(udim));
    if udim {
        w.u32(0x751);
        w.class("DataInt2Box", &[("value", 6)]);
        w.u32(0);
        w.u32(1);
    }
    // Group with a paint layer that has a mask.
    w.u32(0x310);
    w.class(
        "DataLayerGroup",
        &[
            ("actions", 18),
            ("enabled", 10),
            ("label", 16),
            ("maskActions", 18),
            ("perChannelBlending", 17),
            ("subStack", 18),
        ],
    );
    w.u32(0);
    w.out.push(0);
    w.string("Dirt");
    w.u32(0);
    w.u32(0);
    w.u32(0x311);
    w.class("DataStackLayers", &[("items", 19)]);
    w.u32(1);
    w.u32(0x312);
    w.class(
        "DataLayerColor",
        &[
            ("actions", 18),
            ("enabled", 10),
            ("label", 16),
            ("maskActions", 18),
            ("perChannelBlending", 17),
            ("geometryMaskType", 9),
            ("enabledMeshDefault", 10),
            ("enabledMeshList", 19),
            ("enabledUVTileDefault", 10),
            ("enabledUVTileList", 19),
        ],
    );
    if strokes || paint_fill || stencil.is_some() || tip.is_some() || pressure {
        write_strokes(&mut w, 0x500, false, stencil, tip, pressure, paint_fill);
    } else if anchors {
        w.u32(0x720);
        w.class("DataStackActions", &[("items", 19)]);
        w.u32(1);
        write_reference(&mut w, 0x722, 1, 16_777_217, false);
    } else {
        w.u32(0x313);
        w.class("DataStackActions", &[("items", 19)]);
        w.u32(0);
    }
    w.out.push(1);
    w.string("Scratches");
    w.u32(0x314); // a mask
    w.class("DataStackActions", &[("items", 19)]);
    if mask_image.is_some() {
        write_generator(&mut w);
    } else if anchors {
        w.u32(2);
        write_reference(&mut w, 0x732, 0, 16_777_218, true);
        write_reference(&mut w, 0x742, 0, 16_777_219, false);
    } else {
        w.u32(0);
    }
    w.u32(0);
    w.u32(0);
    w.out.push(1);
    w.u32(0);
    w.out.push(1);
    w.u32(0);
    // The material's tile declarations follow its stacks.
    w.out.push(u8::from(udim));
    w.u32(if udim { 2 } else { 0 });
    if udim {
        for v in 0..2 {
            w.u32(0x760 + v);
            w.class(
                "DataUVTile",
                &[
                    ("active", 10),
                    ("indices", 6),
                    ("resolutionLog2", 6),
                    ("resolutionOverridden", 10),
                ],
            );
            w.out.push(1);
            w.u32(0);
            w.u32(v);
            w.u32(7);
            w.u32(7);
            w.out.push(u8::from(v != 0));
        }
    }
    // The document's last field.
    w.u32(normal_syscoord as u32);
    w.out
}

fn write_anchor(w: &mut DocWriter, id: u32, uid: u32) {
    w.u32(id);
    w.class("DataActionAnchor", &[("uid", 11)]);
    w.u32(uid);
}

fn write_uniform(w: &mut DocWriter, id: u32, gray: f32) {
    w.u32(id);
    w.class("DataActionFill", &[("channelTypes", 12), ("sources", 19)]);
    w.out.extend(u64::MAX.to_le_bytes());
    w.u32(1);
    w.u32(id + 1);
    w.class("DataSourceUniform", &[("channelTypes", 12), ("color", 3)]);
    w.out.extend(1u64.to_le_bytes());
    for _ in 0..3 {
        w.u32(gray.to_bits());
    }
}

fn write_reference(w: &mut DocWriter, id: u32, channels: u64, anchor: u32, levels: bool) {
    w.u32(id);
    w.class("DataActionFill", &[("channelTypes", 12), ("sources", 19)]);
    w.out.extend(u64::MAX.to_le_bytes());
    w.u32(1);
    w.u32(id + 1);
    w.class(
        "DataSourceReference",
        &[("channelTypes", 12), ("reference", 18), ("levels", 18)],
    );
    w.out.extend(channels.to_le_bytes());
    w.u32(id + 2);
    w.class("DataReference", &[("source", 15)]);
    w.u32(anchor);
    w.u32(0);
    if levels {
        w.u32(id + 3);
        w.class("DataLevels", &[("gamma", 3)]);
        for _ in 0..3 {
            w.u32(2f32.to_bits());
        }
    } else {
        w.u32(0);
    }
}

fn write_mask_effects(w: &mut DocWriter, url: &str, outline: bool) {
    w.u32(0x600);
    w.class("DataStackActions", &[("items", 19)]);
    w.u32(6 + u32::from(outline));
    w.u32(0x601);
    w.class("DataActionFill", &[("channelTypes", 12), ("sources", 19)]);
    w.out.extend(u64::MAX.to_le_bytes());
    w.u32(1);
    w.u32(0x602);
    w.class("DataSourceBitmap", &[("channelTypes", 12), ("bitmap", 18)]);
    w.out.extend(u64::MAX.to_le_bytes());
    w.u32(0x603);
    w.class("DataBitmap", &[("urlToBitmapRes", 16)]);
    w.string(url);
    w.u32(0x604);
    w.class("DataActionFilterLevels", &[("levels", 18)]);
    w.u32(0x607);
    w.class("DataLevels", &[("gamma", 3)]);
    for _ in 0..3 {
        w.u32(2f32.to_bits());
    }
    w.u32(0x608);
    w.class("DataActionFilterProcedural", &[("procedural", 18)]);
    w.u32(0x609);
    w.class("DataProcedural", &[("urlToSbsRes", 16), ("tweaks", 19)]);
    w.string("starter/blur/blur");
    w.u32(2);
    for (id, key, value) in [(0x60a, "intensity", 0.5_f32), (0x60b, "blur_tilling", 1.)] {
        w.u32(id);
        w.class("DataTweakFloat", &[("identifier", 16), ("value", 1)]);
        w.string(key);
        w.u32(value.to_bits());
    }
    w.u32(0x60c);
    w.class("DataActionFilterProcedural", &[("procedural", 18)]);
    w.u32(0x60d);
    w.class("DataProcedural", &[("urlToSbsRes", 16), ("tweaks", 19)]);
    w.string("starter/sharpen/sharpen");
    w.u32(1);
    w.u32(0x60e);
    w.class("DataTweakFloat", &[("identifier", 16), ("value", 1)]);
    w.string("intensity");
    w.u32(0.25_f32.to_bits());
    for (id, source) in [(0x620, 0.0_f32), (0x630, 2.)] {
        w.u32(id);
        w.class("DataActionFilterProcedural", &[("procedural", 18)]);
        w.u32(id + 1);
        w.class("DataProcedural", &[("urlToSbsRes", 16), ("tweaks", 19)]);
        w.string("starter/warp/warp");
        let tweaks = [
            ("source_type", source),
            ("intensity", 0.078),
            ("intensity_divider", 10.),
            ("noise_scale", 0.5),
            ("$randomseed", 7.),
        ];
        w.u32(tweaks.len() as u32);
        for (index, (key, value)) in tweaks.into_iter().enumerate() {
            w.u32(id + 2 + index as u32);
            w.class("DataTweakFloat", &[("identifier", 16), ("value", 1)]);
            w.string(key);
            w.u32(value.to_bits());
        }
    }
    if outline {
        w.u32(0x640);
        w.class("DataActionFilterProcedural", &[("procedural", 18)]);
        w.u32(0x641);
        w.class("DataProcedural", &[("urlToSbsRes", 16), ("tweaks", 19)]);
        w.string("starter/mask_outline/mask_outline");
        let tweaks = [
            ("input_threshold", 0.75_f32),
            ("width", 0.025),
            ("mask_position", 1.),
            ("blur", 0.01),
            ("curve", -0.25),
            ("switch", 0.),
        ];
        w.u32(tweaks.len() as u32);
        for (index, (key, value)) in tweaks.into_iter().enumerate() {
            w.u32(0x642 + index as u32);
            w.class("DataTweakFloat", &[("identifier", 16), ("value", 1)]);
            w.string(key);
            w.u32(value.to_bits());
        }
    }
}

fn write_generator(w: &mut DocWriter) {
    w.u32(1);
    w.u32(0x610);
    w.class("DataActionFill", &[("channelTypes", 12), ("sources", 19)]);
    w.out.extend(u64::MAX.to_le_bytes());
    w.u32(1);
    w.u32(0x611);
    w.class(
        "DataSourceProcedural",
        &[("channelTypes", 12), ("procedural", 18)],
    );
    w.out.extend(u64::MAX.to_le_bytes());
    w.u32(0x612);
    w.class("DataProcedural", &[("urlToSbsRes", 16), ("tweaks", 19)]);
    w.string("starter/mg_dirt/mg_dirt");
    w.u32(0);
}

fn write_polygon_mask(w: &mut DocWriter) {
    w.u32(0x320);
    w.class("DataStackActions", &[("items", 19)]);
    w.u32(1);
    w.u32(0x321);
    w.class("DataActionDecal", &[("hits", 19)]);
    w.u32(2);
    w.u32(0x322);
    w.class(
        "DataDecalHitPunctual",
        &[
            ("position", 2),
            ("color", 3),
            ("granularity", 9),
            ("viewType", 9),
            ("projViewInvMatrix", 14),
        ],
    );
    for f in [0.75f32, 0.25, 0.25, 0.25, 0.25] {
        w.u32(f.to_bits());
    }
    w.u32(0); // triangle
    w.u32(0); // UV view
    for f in bevy::math::Mat4::IDENTITY.to_cols_array() {
        w.u32(f.to_bits());
    }
    w.u32(0x323);
    w.class(
        "DataDecalHitRect",
        &[
            ("position0", 2),
            ("position1", 2),
            ("color", 3),
            ("granularity", 9),
            ("viewType", 9),
            ("projViewInvMatrix", 14),
        ],
    );
    for f in [0.62f32, 0.79, 0.72, 0.88, 0.75, 0.75, 0.75] {
        w.u32(f.to_bits());
    }
    w.u32(0);
    w.u32(1); // 3D view
    for f in bevy::math::Mat4::IDENTITY.to_cols_array() {
        w.u32(f.to_bits());
    }
}

fn write_strokes(
    w: &mut DocWriter,
    id: u32,
    mask: bool,
    stencil: Option<(&str, bool, Option<bool>)>,
    tip: Option<(&str, TipStroke)>,
    pressure: bool,
    initial_fill: bool,
) {
    w.u32(id);
    w.class("DataStackActions", &[("items", 19)]);
    let single = stencil.is_some() || tip.is_some() || pressure;
    let actions = if single {
        1
    } else if mask {
        2
    } else {
        3
    };
    w.u32(actions + u32::from(initial_fill));
    if initial_fill {
        write_uniform(w, id + 50, 0.25);
    }
    for (i, (kind, uv, color, opacity)) in [
        (
            "DataActionPaint",
            true,
            if mask { [0.; 3] } else { [1., 0., 0.] },
            1.,
        ),
        ("DataActionEraser", true, [0.; 3], 0.5),
        ("DataActionPaint", false, [0., 1., 0.], 1.),
    ]
    .into_iter()
    .skip(if single { 2 } else { 0 })
    .take(if mask { 2 } else { 3 })
    .enumerate()
    {
        let tip_stroke = tip.map_or(TipStroke::Plain, |(_, stroke)| stroke);
        let follow_path = matches!(tip_stroke, TipStroke::Follow | TipStroke::FollowTap);
        let uv = uv
            || stencil.is_some_and(|(_, uv, _)| uv)
            || follow_path
            || matches!(tip_stroke, TipStroke::Splatter { uv: true, .. });
        let tip_emitter = matches!(
            tip_stroke,
            TipStroke::Rotated | TipStroke::Follow | TipStroke::FollowTap | TipStroke::Jittered
        );
        let id = id + 1 + i as u32 * 10;
        w.u32(id);
        let mut properties = vec![
            ("brush", 18),
            ("strokes", 19),
            ("opacity", 1),
            ("sourceColor", 19),
        ];
        if stencil.is_some() {
            properties.extend([("sourceStencil", 18), ("projMatrix", 13)]);
        }
        if tip.is_some() {
            properties.push(("sourceTransparent", 19));
        }
        w.class(kind, &properties);
        w.u32(id + 1);
        if pressure || tip_emitter {
            w.class("DataBrush", &[("brushSize", 1), ("stampsEmitter", 18)]);
        } else {
            w.class("DataBrush", &[("brushSize", 1)]);
        }
        w.u32(if uv { 0.3f32 } else { 0.035f32 }.to_bits());
        if pressure {
            w.u32(id + 9);
            w.class(
                "DataEmitterStroke",
                &[
                    ("pressureSizeEnabled", 10),
                    ("pressureSizeMinimum", 1),
                    ("pressureFlowEnabled", 10),
                    ("pressureFlowMinimum", 1),
                ],
            );
            for _ in 0..2 {
                w.out.push(1);
                w.u32(0);
            }
            w.u32(3);
            write_stroke_path(w, id + 100, false, false, &[([0.625, 0.875], 0.)], &[]);
            write_stroke_path(
                w,
                id + 200,
                false,
                false,
                &[([0.875, 0.875], 0.)],
                &[(0., 0.)],
            );
            write_stroke_path(
                w,
                id + 300,
                false,
                false,
                &[([0.75, 0.6], 0.), ([0.75, 0.7], 0.25), ([0.75, 0.8], 1.)],
                &[(0., 0.), (1., 1.)],
            );
        } else {
            if tip_emitter {
                w.u32(id + 9);
                w.class(
                    "DataEmitterStroke",
                    &[
                        ("rotation", 1),
                        ("followPath", 10),
                        ("rotationJitter", 1),
                        ("positionJitter", 1),
                    ],
                );
                let angle = if tip_stroke == TipStroke::Rotated {
                    std::f32::consts::FRAC_PI_2
                } else {
                    0.
                };
                w.u32(angle.to_bits());
                w.out.push(u8::from(follow_path));
                let jitter = if tip_stroke == TipStroke::Jittered {
                    [1.2_f32, 0.5]
                } else {
                    [0.; 2]
                };
                for value in jitter {
                    w.u32(value.to_bits());
                }
            }
            w.u32(1);
            let point = if uv {
                [0.75, 0.25]
            } else if stencil.is_some_and(|(_, _, inverted)| inverted.is_none()) {
                [0.63, 0.875]
            } else {
                [0.625, 0.875]
            };
            let path = [(point, 0.), ([0.75, 0.5], 1.)];
            let count = if tip_stroke == TipStroke::Follow {
                2
            } else {
                1
            };
            write_stroke_path(
                w,
                id + 100,
                uv,
                uv && stencil.is_some(),
                &path[..count],
                &[],
            );
        }
        w.u32(f32::to_bits(opacity));
        w.u32(1);
        w.u32(id + 4);
        w.class("DataSourceUniform", &[("channelTypes", 12), ("color", 3)]);
        w.out.extend(1u64.to_le_bytes());
        for f in color {
            w.u32(f32::to_bits(f));
        }
        if let Some((url, uv, inverted)) = stencil {
            w.u32(id + 5);
            let matrix = if let Some(inverted) = inverted {
                w.class(
                    "DataSourceProcedural",
                    &[("channelTypes", 12), ("procedural", 18)],
                );
                w.out.extend(1u64.to_le_bytes());
                w.u32(id + 6);
                w.class("DataProcedural", &[("urlToSbsRes", 16), ("tweaks", 19)]);
                w.string(url);
                w.u32(1);
                w.u32(id + 7);
                w.class("DataTweakFloat", &[("identifier", 16), ("value", 1)]);
                w.string("invert");
                w.u32(f32::from(u8::from(inverted)).to_bits());
                let offset = if uv { [3., 2.5] } else { [1.75, -2.5] };
                [-5f32, 0., 0., 0., 4., 0., offset[0], offset[1], 1.]
            } else {
                w.class("DataSourceBitmap", &[("channelTypes", 12), ("bitmap", 18)]);
                w.out.extend(1u64.to_le_bytes());
                w.u32(id + 6);
                w.class("DataBitmap", &[("urlToBitmapRes", 16)]);
                w.string(url);
                if uv {
                    [-1f32, 0., 0., 0., 1., 0., 0.5, 0.5, 1.]
                } else {
                    [-1f32, 0., 0., 0., 0.5, 0., 0.27, 0.5, 1.]
                }
            };
            for f in matrix {
                w.u32(f.to_bits());
            }
        }
        if let Some((url, stroke)) = tip {
            w.u32(1);
            w.u32(id + 7);
            if let TipStroke::Splatter {
                inverted,
                horizontal,
                ..
            } = stroke
            {
                w.class(
                    "DataSourceProcedural",
                    &[("channelTypes", 12), ("procedural", 18)],
                );
                w.out.extend(1u64.to_le_bytes());
                w.u32(id + 8);
                w.class("DataProcedural", &[("urlToSbsRes", 16), ("tweaks", 19)]);
                w.string(url);
                w.u32(2);
                for (shift, name, enabled) in
                    [(0, "invert", inverted), (1, "flip_horizontal", horizontal)]
                {
                    w.u32(id + 20 + shift);
                    w.class("DataTweakFloat", &[("identifier", 16), ("value", 1)]);
                    w.string(name);
                    w.u32(f32::from(u8::from(enabled)).to_bits());
                }
            } else {
                w.class("DataSourceBitmap", &[("channelTypes", 12), ("bitmap", 18)]);
                w.out.extend(1u64.to_le_bytes());
                w.u32(id + 8);
                w.class("DataBitmap", &[("urlToBitmapRes", 16)]);
                w.string(url);
            }
        }
    }
}

fn write_stroke_path(
    w: &mut DocWriter,
    id: u32,
    uv: bool,
    mirror: bool,
    points: &[([f32; 2], f32)],
    pressure: &[(f32, f32)],
) {
    use bevy::math::{Mat4, Vec3};
    w.u32(id);
    let mut properties = vec![
        ("viewType", 9),
        ("points", 19),
        ("viewMatrix", 14),
        ("projectionMatrix", 14),
        ("pressure", 19),
    ];
    if mirror {
        properties.extend([("duplicateCount", 9), ("duplicateMatrix", 14)]);
    }
    w.class("DataStroke", &properties);
    w.u32(u32::from(!uv));
    w.u32(points.len() as u32);
    for (i, &(point, time)) in points.iter().enumerate() {
        w.u32(id + 1 + i as u32);
        w.class("DataStrokePoint", &[("position", 2), ("time", 1)]);
        for f in [point[0], point[1], time] {
            w.u32(f.to_bits());
        }
    }
    let matrices = if uv {
        [Mat4::ZERO; 2]
    } else {
        let gl_depth = Mat4::from_cols_array(&[
            1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 2., 0., 0., 0., -1., 1.,
        ]);
        [
            Mat4::from_translation(-Vec3::Z),
            gl_depth * bevy::math::proj::perspective(std::f32::consts::FRAC_PI_2, 1., 0.1, 10.),
        ]
    };
    for matrix in matrices {
        for f in matrix.to_cols_array() {
            w.u32(f.to_bits());
        }
    }
    w.u32(pressure.len() as u32);
    for (i, &(time, value)) in pressure.iter().enumerate() {
        w.u32(id + 1 + points.len() as u32 + i as u32);
        w.class("DataStrokePressure", &[("time", 1), ("value", 1)]);
        w.u32(time.to_bits());
        w.u32(value.to_bits());
    }
    if mirror {
        w.u32(1);
        for f in Mat4::from_scale(Vec3::new(-1., 1., 1.)).to_cols_array() {
            w.u32(f.to_bits());
        }
    }
}

/// An Assimp-style `scene.assbin` with one quad (two triangles, UVs covering the unit square).
pub fn assbin_quad() -> Vec<u8> {
    assbin_quads(&[("Cube", 0., 1., 0., 0.)])
}

pub fn assbin_two_parts() -> Vec<u8> {
    assbin_quads(&[("Left", 0., 0.5, 0., 0.), ("Right", 0.5, 1., 0., 0.)])
}

pub fn assbin_udim() -> Vec<u8> {
    assbin_quads(&[("Lower", 0., 1., 0., 0.), ("Upper", 0., 1., 1., 0.)])
}

pub fn assbin_quad_with_margin() -> Vec<u8> {
    assbin_quads(&[("Cube", 0.25, 0.75, 0., 0.)])
}

pub fn assbin_overlapping_charts(front_last: bool) -> Vec<u8> {
    let front = ("Front", 0., 1., 0., 0.);
    let back = ("Back", 0., 1., 0., -0.5);
    assbin_quads(&if front_last {
        [back, front]
    } else {
        [front, back]
    })
}

fn assbin_quads(parts: &[(&str, f32, f32, f32, f32)]) -> Vec<u8> {
    let mut b = b"ASSIMP.binary-dump.test".to_vec();
    b.resize(524, 0);
    b.extend(4u32.to_le_bytes()); // node data length (counted from the length field itself)
    for (i, &(name, left, right, uv_v, depth)) in parts.iter().enumerate() {
        b.extend(assbin_quad_record(
            name,
            left,
            right,
            uv_v,
            depth,
            if i == 0 { parts.len() as u32 } else { 0 },
        ));
    }
    b
}

fn assbin_quad_record(
    name: &str,
    left: f32,
    right: f32,
    uv_v: f32,
    depth: f32,
    count: u32,
) -> Vec<u8> {
    let mut rec = Vec::new();
    rec.extend(count.to_le_bytes()); // the first record starts with the mesh count
    rec.extend(0u32.to_le_bytes()); // length, patched below
    rec.extend(0u32.to_le_bytes());
    rec.extend((name.len() as u32).to_le_bytes());
    rec.extend(name.as_bytes());
    rec.extend(4u32.to_le_bytes()); // triangles
    rec.extend(0u32.to_le_bytes());
    rec.extend((1u32 | 2 | 0x100).to_le_bytes());
    let corners = [
        [left, 0.0, 0.0],
        [right, 0.0, 0.0],
        [right, 1.0, 0.0],
        [left, 1.0, 0.0],
    ];
    let floats = |rec: &mut Vec<u8>, data: &[[f32; 3]]| {
        rec.extend((data.len() as u32).to_le_bytes());
        for v in data {
            for c in v {
                rec.extend(c.to_le_bytes());
            }
        }
    };
    floats(&mut rec, &corners.map(|[x, y, _]| [x, y, depth]));
    floats(&mut rec, &[[0.0, 0.0, 1.0]; 4]);
    rec.extend(3u32.to_le_bytes());
    floats(&mut rec, &corners.map(|[x, y, z]| [x, y + uv_v, z]));
    rec.extend(2u32.to_le_bytes());
    for t in [[0u32, 1, 2], [0, 2, 3]] {
        rec.extend(0u32.to_le_bytes());
        rec.extend(3u32.to_le_bytes());
        for i in t {
            rec.extend(i.to_le_bytes());
        }
    }
    let len = rec.len() as u32;
    rec[4..8].copy_from_slice(&len.to_le_bytes());
    rec
}
