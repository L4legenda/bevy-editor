//! Imports a grayscale layer mask without changing the source image or layer pixels.
use super::*;
use photocraft_doc::{DocId, LayerMask};
use photocraft_geom::{Rect, Size};
use std::{io, path::Path, process::Command, sync::mpsc};

pub(super) struct Import {
    tab: usize,
    document: DocId,
    layer: LayerId,
    size: Size,
    result: std::sync::Mutex<mpsc::Receiver<Result<Option<Surface>, String>>>,
}

pub(super) fn can_add(doc: &PhotoDocument, id: LayerId) -> bool {
    doc.document
        .layer(id)
        .is_some_and(|layer| layer.mask.is_none())
        && !doc.document.effective_locks(id).all
}

pub(super) fn start(state: &mut State, tab: usize) {
    if state.layers.mask_import.is_some() || state.file_busy(tab) {
        return;
    }
    let Some(doc) = state.document(Some(tab)) else {
        return;
    };
    let Some(layer) = doc.active_id().filter(|&id| can_add(doc, id)) else {
        return;
    };
    let (document, size) = (doc.document.id, doc.size());
    let (tx, rx) = mpsc::channel();
    if let Err(error) = std::thread::Builder::new()
        .name("image-mask".into())
        .spawn(move || {
            let result = choose_image()
                .map_err(|error| format!("Не удалось открыть выбор изображения: {error}"))
                .and_then(|path| path.map(|path| read_mask(&path, size)).transpose());
            let _ = tx.send(result);
        })
    {
        state.error = Some(format!("Не удалось запустить импорт маски: {error}"));
        return;
    }
    state.layers.mask_import = Some(Import {
        tab,
        document,
        layer,
        size,
        result: rx.into(),
    });
    state.layers.menu = false;
    state.layers.context_menu_y = None;
    state.layers.form = None;
    state.layers.ui_revision = state.layers.ui_revision.wrapping_add(1);
    state.error = None;
}

pub(in crate::editor) fn poll(state: &mut State, editor: &mut Editor) {
    let Some(import) = state.layers.mask_import.as_mut() else {
        return;
    };
    if state.layers.preview.is_some()
        || state
            .saving
            .as_ref()
            .is_some_and(|save| save.id == import.tab)
    {
        return;
    }
    let result = match import.result.get_mut().ok().map(|rx| rx.try_recv()) {
        Some(Ok(result)) => result,
        Some(Err(mpsc::TryRecvError::Empty)) => return,
        _ => Err("Импорт маски прервался.".into()),
    };
    let Some(import) = state.layers.mask_import.take() else {
        return;
    };
    let Some(session) = state
        .sessions
        .get_mut(&import.tab)
        .filter(|session| session.editing && session.doc.document.id == import.document)
    else {
        return;
    };
    match result {
        Ok(Some(surface)) => {
            let doc = &mut session.doc;
            if doc.size() != import.size || !apply(doc, import.layer, surface) {
                state.error = Some(
                    "Слой или размеры холста изменились. Откройте импорт маски заново.".into(),
                );
                return;
            }
            if editor.viewing_image == Some(import.tab) {
                editor.status = "Маска добавлена из изображения".into();
                state.error = None;
            }
        }
        Ok(None) => {}
        Err(error) => state.error = Some(error),
    }
}

fn apply(doc: &mut PhotoDocument, id: LayerId, surface: Surface) -> bool {
    if !can_add(doc, id) {
        return false;
    }
    doc.checkpoint();
    if let Some(layer) = doc.document.layer_mut(id) {
        layer.mask = Some(LayerMask {
            surface,
            ..LayerMask::reveal_all()
        });
    }
    doc.changed();
    true
}

fn read_mask(path: &Path, size: Size) -> Result<Surface, String> {
    use image::ImageDecoder;
    let mut reader = image::ImageReader::open(path)
        .and_then(image::ImageReader::with_guessed_format)
        .map_err(|error| format!("Не удалось открыть изображение маски: {error}"))?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(256 << 20);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().map_err(|error| error.to_string())?;
    let (width, height) = decoder.dimensions();
    if u64::from(width) * u64::from(height) > photo_document::MAX_PIXELS {
        return Err("Изображение маски должно быть не больше 16 Мп.".into());
    }
    let orientation = decoder.orientation().map_err(|error| error.to_string())?;
    let mut image =
        image::DynamicImage::from_decoder(decoder).map_err(|error| error.to_string())?;
    image.apply_orientation(orientation);
    // Treat alpha as coverage before resampling, so hidden RGB cannot bleed into the mask.
    let gray = image.to_luma_alpha8();
    let gray = image::GrayImage::from_fn(gray.width(), gray.height(), |x, y| {
        let [light, alpha] = gray.get_pixel(x, y).0;
        image::Luma([((u16::from(light) * u16::from(alpha) + 127) / 255) as u8])
    });
    let fitted = if gray.dimensions() == (size.width, size.height) {
        gray
    } else {
        image::imageops::resize(
            &gray,
            size.width,
            size.height,
            image::imageops::FilterType::Triangle,
        )
    };
    let mut surface = Surface::new(photocraft_color::PixelFormat::GRAY8);
    surface.write_interleaved(Rect::from_size(size), fitted.as_raw());
    Ok(surface)
}

fn choose_image() -> io::Result<Option<PathBuf>> {
    #[cfg(target_os = "windows")]
    let output = {
        use std::os::windows::process::CommandExt;
        Command::new("powershell.exe").creation_flags(0x0800_0000)
            .args(["-NoProfile", "-STA", "-Command", concat!(
                "[Console]::OutputEncoding = [System.Text.Encoding]::UTF8; ",
                "Add-Type -AssemblyName System.Windows.Forms; ",
                "$dialog = New-Object System.Windows.Forms.OpenFileDialog; ",
                "$dialog.Title = 'Bevy Editor: image mask'; ",
                "$dialog.Filter = 'Images|*.png;*.jpg;*.jpeg;*.webp;*.bmp;*.tga;*.gif;*.ico|All files|*.*'; ",
                "if ($dialog.ShowDialog() -eq 'OK') { [Console]::Write($dialog.FileName) }; $dialog.Dispose()"
            )]).output()?
    };
    #[cfg(target_os = "macos")]
    let output = Command::new("/usr/bin/osascript").args(["-e",
        "try\nreturn POSIX path of (choose file with prompt \"Выберите изображение маски\")\non error number -128\nreturn \"\"\nend try"
    ]).output()?;
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let output = match Command::new("zenity")
        .args(["--file-selection", "--title=Изображение маски"])
        .output()
    {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Command::new("kdialog")
            .args(["--getopenfilename", "."])
            .output()?,
        result => result?,
    };
    if !output.status.success() {
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        if output.status.code() == Some(1) {
            return Ok(None);
        }
        return Err(io::Error::other(
            String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        ));
    }
    let value = String::from_utf8(output.stdout).map_err(io::Error::other)?;
    let path = value.trim_end_matches(['\r', '\n']);
    Ok((!path.is_empty()).then(|| PathBuf::from(path)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bitmap_mask_uses_brightness_and_alpha_and_is_undoable() {
        let path = std::env::temp_dir().join(format!("bevy-image-mask-{}.png", std::process::id()));
        image::RgbaImage::from_fn(4, 1, |x, _| {
            image::Rgba(match x {
                0 => [255, 255, 255, 255],
                1 => [0, 0, 0, 255],
                2 => [255, 255, 255, 0],
                _ => [255, 255, 255, 128],
            })
        })
        .save(&path)
        .unwrap();
        let resized = read_mask(&path, Size::new(8, 2)).unwrap();
        assert_eq!(resized.sample_channel(0, 1, 0), 1.0);
        assert!((resized.sample_channel(7, 1, 0) - 128.0 / 255.0).abs() < 0.001);
        let surface = read_mask(&path, Size::new(4, 1)).unwrap();
        for (x, expected) in [255, 0, 0, 128].into_iter().enumerate() {
            assert_eq!(
                (surface.sample_channel(x as i32, 0, 0) * 255.0).round() as u8,
                expected
            );
        }
        image::RgbaImage::from_pixel(4, 1, image::Rgba([30, 60, 90, 255]))
            .save(&path)
            .unwrap();
        let mut doc = PhotoDocument::load(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("pcraft"));
        let layer = doc.active_id().unwrap();
        assert!(apply(&mut doc, layer, surface));
        assert!(!can_add(&doc, layer));
        let image = doc.composite();
        assert_eq!(
            image
                .pixels
                .chunks_exact(4)
                .map(|p| p[3])
                .collect::<Vec<_>>(),
            [255, 0, 0, 128]
        );
        doc.undo();
        assert!(doc.active_layer().unwrap().mask.is_none());
        assert!(!doc.can_undo());
    }
}
