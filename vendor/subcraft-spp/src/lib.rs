//! Read the geometry, layer metadata and texture data of a Substance Painter project.
pub mod assbin;
pub mod document;
mod hdf5;
pub mod images;
mod jp2;
mod lz4;
mod qt;

use std::{
    fs::File,
    io::{self, BufReader, Read, Seek},
    path::Path,
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Container(#[from] hdf5::Hdf5Error),
    #[error(transparent)]
    Mesh(#[from] assbin::AssbinError),
    #[error(transparent)]
    Document(#[from] document::DocumentError),
    #[error(transparent)]
    Image(#[from] images::ImageError),
    #[error(transparent)]
    Io(#[from] io::Error),
}

pub struct FileReader<R: Read + Seek = BufReader<File>> {
    container: hdf5::Hdf5File<R>,
}

fn tile_cache_key([u, v]: [u32; 2]) -> u64 {
    // The saved key combines V into U with the 64-bit Murmur2/Boost combiner.
    // On-disk wrapping is fixed at 64 bits, independent of the reader's host.
    const MIX: u64 = 0xc6a4_a793_5bd1_e995;
    let value = u64::from(v).wrapping_mul(MIX);
    let value = (value ^ (value >> 47)).wrapping_mul(MIX);
    (u64::from(u) ^ value).wrapping_mul(MIX).wrapping_add(0xe654_6b64)
}

impl FileReader<BufReader<File>> {
    pub fn open(path: &Path) -> Result<Self, Error> {
        Self::new(BufReader::new(File::open(path)?))
    }
}

impl<R: Read + Seek> FileReader<R> {
    pub fn new(reader: R) -> Result<Self, Error> {
        Ok(Self {
            container: hdf5::Hdf5File::open(reader)?,
        })
    }

    fn bytes(&mut self, path: &str) -> Result<Vec<u8>, Error> {
        let dataset = self.container.dataset(path)?;
        self.read(&dataset)
    }

    fn read(&mut self, dataset: &hdf5::DatasetInfo) -> Result<Vec<u8>, Error> {
        if dataset.len > 512 * 1024 * 1024 {
            return Err(io::Error::other("SPP dataset exceeds 512 MiB").into());
        }
        Ok(self.container.read(dataset)?)
    }

    pub fn meshes(&mut self) -> Result<(Vec<assbin::DumpMesh>, Vec<String>), Error> {
        let bytes = self.bytes("paint/scene.assbin")?;
        Ok(assbin::scene(&bytes)?)
    }

    pub fn project(&mut self) -> Result<document::ProjectInfo, Error> {
        let project = document::parse(&self.bytes("paint/document.bin")?)?;
        Ok(document::project_info(&project))
    }

    pub fn channel(
        &mut self,
        set: &document::TextureSetInfo,
        bits: u64,
        max: u32,
    ) -> Result<Option<images::CacheImage>, Error> {
        let path = format!("texture-cache/{}", set.uid);
        let Ok(entries) = self.container.list(&path) else {
            return Ok(None);
        };
        for entry in entries {
            if let Ok(dataset) = self.container.dataset(&format!("{path}/{entry}/{bits}")) {
                return Ok(Some(images::decode_cache_image(
                    &self.read(&dataset)?,
                    max,
                )?));
            }
        }
        Ok(None)
    }

    /// Read the cache group belonging to this exact UV tile.
    pub fn tile_channel(
        &mut self,
        set: &document::TextureSetInfo,
        tile: [u32; 2],
        bits: u64,
        max: u32,
    ) -> Result<Option<images::CacheImage>, Error> {
        let key = tile_cache_key(tile);
        let path = format!("texture-cache/{}/{key}/{bits}", set.uid);
        let Ok(dataset) = self.container.dataset(&path) else { return Ok(None) };
        Ok(Some(images::decode_cache_image(&self.read(&dataset)?, max)?))
    }

    pub fn bitmap(&mut self, url: &str, max: u32) -> Result<Option<images::Plane>, Error> {
        let Some((_, key)) = url.split_once("?version=") else {
            return Ok(None);
        };
        let Some(id) = key.strip_suffix(".image") else {
            return Ok(None);
        };
        if id.len() != 40 || !id.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Ok(None);
        }
        let Ok(dataset) = self.container.dataset(&format!("resources/{key}")) else {
            return Ok(None);
        };
        Ok(Some(images::decode_image_resource(
            &self.read(&dataset)?,
            max,
        )?))
    }
}
