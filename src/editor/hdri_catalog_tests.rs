use super::*;
use serde_json::json;

#[test]
fn catalogs_keep_only_hdris_and_resolve_current_provider_formats() {
    let poly = parse_polyhaven(&json!({
        "studio": {"type":0,"name":"Small Studio","tags":["soft light"],"download_count":20},
        "forest": {"type":0,"name":"Forest","download_count":100},
        "texture": {"type":1,"name":"Texture"},
        "../escape": {"type":0,"name":"Invalid"}
    }))
    .unwrap();
    assert_eq!(poly.len(), 2);
    assert_eq!(poly[0].id, "forest");
    assert!(poly[1].matches("STUDIO soft"));
    assert!(!poly[1].matches("studio forest"));
    let ambient = parse_ambient(&json!({"assets":[{
        "id":"DaySkyHDRI072B", "title":"Day Sky", "tags":["sky"],
        "thumbnails":{"256-WEBP":"https://acg-media.struffelproductions.com/sky.webp"},
        "downloads":[
            {"attributes":"2K","extension":"zip","url":"https://ambientcg.com/get?file=sky.zip","size":123},
            {"attributes":"12K","extension":"zip","url":"https://ambientcg.com/get?file=big.zip","size":999}
        ]
    }]})).unwrap();
    assert_eq!(ambient[0].downloads.len(), 1);
    assert_eq!(ambient[0].downloads[0].resolution, 2048);
    assert_eq!(ambient[0].downloads[0].extension, "zip");
}

#[test]
fn archive_import_reads_hdr_or_exr_without_extracting_other_files() {
    use std::io::Write;
    use zip::write::SimpleFileOptions;
    let mut archive = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    archive.start_file("scene.blend", options).unwrap();
    archive.write_all(b"unrelated scene").unwrap();
    archive
        .start_file("../../panorama_HDR.exr", options)
        .unwrap();
    archive.write_all(b"EXR pixels").unwrap();
    let bytes = archive.finish().unwrap().into_inner();
    let (pixels, extension) = unpack(bytes, "zip", &Progress::default()).unwrap();
    assert_eq!(pixels, b"EXR pixels");
    assert_eq!(extension, "exr");
}

#[test]
fn downloads_are_bounded_and_cancel_before_network_or_writes() {
    assert!(bounded_read(Cursor::new(vec![0_u8; 100]), 99, None).is_err());
    let progress = Arc::new(Progress::default());
    progress.cancel();
    assert!(bounded_read(Cursor::new(vec![0_u8; 1]), 99, Some(&progress)).is_err());
    let entry = CatalogEntry {
        provider: HdriProvider::PolyHaven,
        id: "test".into(),
        name: "Test".into(),
        thumbnail: String::new(),
        tags: Vec::new(),
        downloads: Vec::new(),
        popularity: 0,
    };
    assert_eq!(
        download(Path::new("does-not-exist"), &entry, 1024, progress).unwrap_err(),
        "Загрузка отменена"
    );
    assert!(!allowed_url(
        &reqwest::Url::parse("http://api.polyhaven.com/assets").unwrap()
    ));
    assert!(!allowed_url(
        &reqwest::Url::parse("https://localhost/test.hdr").unwrap()
    ));
}

#[test]
fn downloaded_library_survives_reopening_and_ignores_invalid_records() {
    let root = std::env::temp_dir().join(format!(
        "bevy-hdri-catalog-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&root).unwrap();
    let path = directory(&root, Path::new("hdri/polyhaven/studio/2k")).unwrap();
    fs::write(path.join("environment.hdr"), b"test fixture").unwrap();
    let record = LibraryRecord {
        environment: HdriAsset {
            provider: HdriProvider::PolyHaven,
            id: "studio".into(),
            name: "Studio".into(),
            resolution: 2048,
            asset: "hdri/polyhaven/studio/2k/environment.hdr".into(),
        },
        preview: "hdri/polyhaven/studio/2k/preview.png".into(),
        source_url: "https://polyhaven.com/a/studio".into(),
        license: "CC0-1.0".into(),
    };
    atomic_write(
        &path.join("environment.json"),
        &serde_json::to_vec(&record).unwrap(),
    )
    .unwrap();
    let reopened = scan(&root).unwrap();
    assert_eq!(reopened.len(), 1);
    assert_eq!(reopened[0].environment, record.environment);
    let mut invalid = record;
    invalid.environment.asset = "../escape.hdr".into();
    atomic_write(
        &path.join("environment.json"),
        &serde_json::to_vec(&invalid).unwrap(),
    )
    .unwrap();
    assert!(scan(&root).unwrap().is_empty());
    assert!(directory(&root, Path::new("../escape")).is_err());
}

#[test]
#[ignore = "downloads a 1K sample from each public HDRI provider"]
fn live_polyhaven_and_ambientcg_download_and_reopen() {
    let root = std::env::temp_dir().join(format!("bevy-hdri-live-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    for (provider, id) in [
        (HdriProvider::PolyHaven, "studio_small_09"),
        (HdriProvider::AmbientCg, "DaySkyHDRI072B"),
    ] {
        let (entries, _) = catalog(&root, provider, true).unwrap();
        let entry = entries.iter().find(|entry| entry.id == id).unwrap();
        let record = download(&root, entry, 1024, Arc::new(Progress::default())).unwrap();
        assert!(root.join(&record.environment.asset).is_file());
        println!(
            "{}: {}",
            provider.label(),
            root.join(&record.environment.asset).display()
        );
    }
    assert_eq!(scan(&root).unwrap().len(), 2);
}
