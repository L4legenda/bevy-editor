use super::*;
use std::{net::TcpListener, thread};

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "bevy-model-install-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        if let (Ok(target), Ok(parent)) =
            (self.0.canonicalize(), std::env::temp_dir().canonicalize())
            && target.parent() == Some(parent.as_path())
            && target
                .file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with("bevy-model-install-"))
        {
            let _ = fs::remove_dir_all(target);
        }
    }
}

fn server(
    payload: Vec<u8>,
    cut: Option<usize>,
    wrong_range: bool,
) -> (String, thread::JoinHandle<u64>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut request = Vec::new();
        let mut one = [0; 1];
        while !request.ends_with(b"\r\n\r\n") {
            assert!(request.len() < 16384);
            stream.read_exact(&mut one).unwrap();
            request.push(one[0]);
        }
        let request = String::from_utf8(request).unwrap().to_lowercase();
        let range = request
            .lines()
            .find_map(|line| line.strip_prefix("range: bytes="))
            .expect("Range request");
        let (start, end) = range.split_once('-').unwrap();
        let start = start.parse::<u64>().unwrap();
        let end = end.parse::<u64>().unwrap();
        let body = &payload[start as usize..=end as usize];
        write!(stream,"HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {}-{end}/{}\r\nConnection: close\r\n\r\n",body.len(),start+u64::from(wrong_range),payload.len()).unwrap();
        let body = &body[..cut.unwrap_or(body.len()).min(body.len())];
        let _ = stream.write_all(body);
        start
    });
    (format!("http://{address}"), handle)
}

fn artifact(bytes: &[u8]) -> Artifact {
    let sha: &'static str = Box::leak(format!("{:x}", Sha256::digest(bytes)).into_boxed_str());
    Artifact {
        path: "variant/model.safetensors",
        size: bytes.len() as u64,
        sha,
    }
}

#[test]
fn generation_model_download_resumes_real_http_after_disconnect_and_promotes_only_verified_file() {
    let scratch = Scratch::new();
    let bytes: Vec<u8> = (0..100000).map(|i| (i % 251) as u8).collect();
    let file = artifact(&bytes);
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let progress = Progress::default();
    let (url, worker) = server(bytes.clone(), Some(4321), false);
    assert!(download_file(&client, &url, &scratch.0, file, 10, &progress).is_err());
    assert_eq!(worker.join().unwrap(), 0);
    assert!(!scratch.0.join(file.path).exists());
    assert_eq!(progress.received.load(Ordering::Relaxed), 4331);
    let (url, worker) = server(bytes.clone(), None, false);
    download_file(&client, &url, &scratch.0, file, 10, &progress).unwrap();
    assert_eq!(worker.join().unwrap(), 4321);
    assert_eq!(fs::read(scratch.0.join(file.path)).unwrap(), bytes);
    assert!(!scratch.0.join("variant/model.safetensors.part").exists());
    assert_eq!(progress.received.load(Ordering::Relaxed), file.size + 10);
}

#[test]
fn generation_model_bad_checksum_and_range_never_install_a_model() {
    let scratch = Scratch::new();
    let bytes = vec![23; 8192];
    let file = artifact(&bytes);
    let client = reqwest::blocking::Client::new();
    let (url, worker) = server(vec![24; 8192], None, false);
    assert!(
        download_file(&client, &url, &scratch.0, file, 0, &Progress::default())
            .unwrap_err()
            .contains("сумма")
    );
    worker.join().unwrap();
    assert!(!scratch.0.join(file.path).exists());
    assert!(!scratch.0.join("variant/model.safetensors.part").exists());
    let (url, worker) = server(bytes, None, true);
    assert!(
        download_file(&client, &url, &scratch.0, file, 0, &Progress::default())
            .unwrap_err()
            .contains("диапазон")
    );
    worker.join().unwrap();
    assert!(!scratch.0.join(file.path).exists());
}

#[test]
fn generation_model_cancel_preserves_partial_and_system_cache_ignores_everything() {
    let scratch = Scratch::new();
    let store = Store {
        root: scratch.0.join("system/models"),
    };
    store.prepare().unwrap();
    assert_eq!(
        fs::read_to_string(store.root.join(".gitignore")).unwrap(),
        IGNORE
    );
    assert!(store.model().starts_with(&store.root));
    let file = artifact(&[1; 1024]);
    let path = scratch.0.join("variant/model.safetensors.part");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, [1; 128]).unwrap();
    let progress = Progress::default();
    progress.cancel();
    assert!(
        download_file(
            &reqwest::blocking::Client::new(),
            "http://127.0.0.1:1",
            &scratch.0,
            file,
            0,
            &progress
        )
        .is_err()
    );
    assert_eq!(fs::metadata(path).unwrap().len(), 128);
}

#[test]
fn generation_model_detection_rejects_truncated_html_or_incomplete_safetensors() {
    let scratch = Scratch::new();
    let path = scratch.0.join("model.safetensors");
    let mut header = br#"{"tensor":{"dtype":"F32","shape":[1],"data_offsets":[0,4]}}"#.to_vec();
    while header.len() % 8 != 0 {
        header.push(b' ');
    }
    let mut bytes = (header.len() as u64).to_le_bytes().to_vec();
    bytes.extend(header);
    bytes.extend([0; 4]);
    fs::write(&path, &bytes).unwrap();
    assert!(valid_weights(&path, bytes.len() as u64));
    assert!(!valid_weights(&path, bytes.len() as u64 + 1));
    fs::write(&path, b"<!doctype html>not a model").unwrap();
    assert!(!valid_weights(&path, 25));
    assert!(!installed(&scratch.0));
}

#[test]
#[ignore = "uses the official pinned Hugging Face endpoints; transfers under 60 KB"]
fn generation_model_official_manifest_and_range_endpoint() {
    let client = reqwest::blocking::Client::builder()
        .https_only(true)
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap();
    for artifact in &FILES[..3] {
        let bytes = client
            .get(format!("{REPOSITORY}/{REVISION}/{}", artifact.path))
            .send()
            .unwrap()
            .error_for_status()
            .unwrap()
            .bytes()
            .unwrap();
        assert_eq!(bytes.len() as u64, artifact.size);
        assert_eq!(format!("{:x}", Sha256::digest(bytes)), artifact.sha);
    }
    let mut response = client
        .get(format!(
            "{REPOSITORY}/{REVISION}/{}?download=true",
            FILES[3].path
        ))
        .header(reqwest::header::RANGE, "bytes=0-7")
        .send()
        .unwrap()
        .error_for_status()
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::PARTIAL_CONTENT);
    assert_eq!(
        response
            .headers()
            .get(reqwest::header::CONTENT_RANGE)
            .unwrap(),
        "bytes 0-7/3819958234"
    );
    let mut prefix = [0; 8];
    response.read_exact(&mut prefix).unwrap();
    assert!((2..8 * 1024 * 1024).contains(&u64::from_le_bytes(prefix)));
}
