# HDRI preparation in Rust

Rebuilds the ten bundled CC0 Poly Haven environments. This standalone crate does
not build Bevy and requires no Python, NumPy, Python zstandard package or native
compression library. HDR decoding, filtering, half-float packing and Zstandard
encoding all run in Rust.

```powershell
cargo run --locked --release --manifest-path tools/prepare-hdri/Cargo.toml -- --source G:/material-painter/assets/hdri
```

The command retains the previous options:

| Option | Meaning | Default |
| --- | --- | --- |
| `--source DIR` | Folder containing the ten `<name>_1k.hdr` panoramas | Required |
| `--output DIR` | Destination for all twenty maps | Repository `assets/hdri` |
| `--size N` | Power-of-two reflection face size, 1–4096 | 256 |
| `--samples N` | GGX importance samples | 256 |

`--help` lists the options. Both `--size 256` and `--size=256` are accepted.
Input presence is checked before output is written; paths with spaces work.

The source names and output names are unchanged. For each panorama the tool
creates `<name>.diffuse.ktx2` (32×32, one level) and `<name>.specular.ktx2`
(256×256 with nine roughness levels at defaults). Cube faces retain Vulkan order
`+X, -X, +Y, -Y, +Z, -Z`. Diffuse lighting integrates all source pixels using
second-order spherical harmonics and irradiance/pi; specular lighting uses GGX,
the same Hammersley sequence, bilinear wrapping and trilinear source mips.

The container is still linear BT.709 RGBA16F KTX2 with lossless Zstandard
supercompression, little-endian texels and 8-byte-aligned level data. The Rust
`ruzstd` encoder uses its supported `Fastest` mode instead of the old reference
encoder's level 9. Compressed bytes and file sizes therefore differ, without
compression loss. On the original ten panoramas the new outputs total about
31.5 MiB versus 24.7 MiB. The existing bundled maps do not need regeneration.

Floating-point evaluation can slightly change half-float rounding. Validation
against all twenty pre-migration maps found worst relative RMS texel error
0.00011879 (about 0.012%); KTX2 headers and data-format descriptors matched
exactly. The downloaded-panorama loader has its own runtime processing path.

Tests:

```powershell
cargo test --locked --manifest-path tools/prepare-hdri/Cargo.toml --target-dir target/prepare-hdri-native
```

For comparison against the bundled maps, first bake into a separate directory:

```powershell
cargo run --locked --release --manifest-path tools/prepare-hdri/Cargo.toml --target-dir target/prepare-hdri-native -- --source G:/material-painter/assets/hdri --output target/hdri-rust-qa/maps
$env:PREPARE_HDRI_GENERATED = 'G:/bevy-editor/target/hdri-rust-qa/maps'
cargo test --locked --manifest-path tools/prepare-hdri/Cargo.toml --target-dir target/prepare-hdri-native --test reference -- --ignored --nocapture
```

The normal tests cover constant HDR radiance, cube orientation and seams, source
mip filtering, RGBA16F clamping, KTX2/Zstandard round trips, argument validation,
and all twenty outputs from a CLI process with an empty PATH.
The source panoramas and derived maps remain CC0; see `assets/hdri/LICENSE.txt`.
