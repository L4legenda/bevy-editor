# SubCraft SPP reader

The reader modules are from [SubCraft](https://github.com/Aalzard/subcraft/tree/5adf219b18b5b33bc26ca82bd972af601ab0ed21/crates/mesh/src/spp) at revision `5adf219b18b5b33bc26ca82bd972af601ab0ed21`, with the upstream MIT notice. Bevy Editor supplies its own entry point for mesh geometry, project metadata, texture caches, and bitmap resources.

Bevy Editor's version 0 reader uses the wire format documented in Material Painter's `src/spp/document.rs` at revision `1456397558798e5ef51588270db85bf3c310332f`. Both versions share the SubCraft value model and project extraction.
