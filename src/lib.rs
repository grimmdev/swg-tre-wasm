use std::io::{Cursor, Read};
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub fn init_panic_hook() {
    #[cfg(feature = "console_error_panic_hook")]
    console_error_panic_hook::set_once();
}

#[derive(Serialize, Deserialize)]
pub struct FileEntryInfo {
    pub index: usize,
    pub name: String,
    pub uncompressed_size: u64,
    pub compressed_size: u64,
    pub crc32: u32,
    pub compression_method: u32,
    pub data_start: u64,
}

#[derive(Serialize, Deserialize)]
pub struct TreArchiveSummary {
    pub total_files: usize,
    pub decompressed_size: u64,
    pub record_compression: u32,
    pub name_compression: u32,
}

#[wasm_bindgen]
pub struct TreArchiveHandle {
    archive: swg_tre::TreArchive<Cursor<Vec<u8>>>,
}

#[wasm_bindgen]
impl TreArchiveHandle {
    /// Initialize a new TRE archive reader from raw bytes.
    #[wasm_bindgen(constructor)]
    pub fn new(bytes: &[u8]) -> Result<TreArchiveHandle, JsValue> {
        let cursor = Cursor::new(bytes.to_vec());
        let archive = swg_tre::TreArchive::new(cursor)
            .map_err(|e| JsValue::from_str(&format!("Failed to parse TRE archive: {:?}", e)))?;
        Ok(TreArchiveHandle { archive })
    }

    /// Number of files in the archive
    pub fn file_count(&self) -> usize {
        self.archive.len()
    }

    /// List all file paths in the archive
    pub fn list_files(&self) -> Vec<String> {
        self.archive.file_names().map(|s| s.to_string()).collect()
    }

    /// Extract raw file contents by name (handles both forward and backward slashes)
    pub fn extract_file(&mut self, filename: &str) -> Result<js_sys::Uint8Array, JsValue> {
        let mut file = match self.archive.by_name(filename) {
            Ok(f) => f,
            Err(_) => {
                let swapped = if filename.contains('/') {
                    filename.replace('/', "\\")
                } else {
                    filename.replace('\\', "/")
                };
                self.archive.by_name(&swapped)
                    .map_err(|e| JsValue::from_str(&format!("File not found '{filename}': {:?}", e)))?
            }
        };
        
        let mut data = Vec::with_capacity(file.size() as usize);
        file.read_to_end(&mut data)
            .map_err(|e| JsValue::from_str(&format!("Failed to decompress file: {:?}", e)))?;
        
        Ok(js_sys::Uint8Array::from(&data[..]))
    }

    /// Extract raw file contents by index
    pub fn extract_file_by_index(&mut self, index: usize) -> Result<js_sys::Uint8Array, JsValue> {
        let mut file = self.archive.by_index(index)
            .map_err(|e| JsValue::from_str(&format!("File index out of range: {:?}", e)))?;
        
        let mut data = Vec::with_capacity(file.size() as usize);
        file.read_to_end(&mut data)
            .map_err(|e| JsValue::from_str(&format!("Failed to decompress file: {:?}", e)))?;
        
        Ok(js_sys::Uint8Array::from(&data[..]))
    }

    /// Get detailed summary of the archive
    pub fn get_summary(&self) -> Result<JsValue, JsValue> {
        let summary = TreArchiveSummary {
            total_files: self.archive.len(),
            decompressed_size: self.archive.decompressed_size().unwrap_or(0) as u64,
            record_compression: self.archive.get_record_compression() as u32,
            name_compression: self.archive.get_name_compression() as u32,
        };
        serde_wasm_bindgen::to_value(&summary)
            .map_err(|e| JsValue::from_str(&format!("Serialization error: {:?}", e)))
    }

    /// Get file metadata by index
    pub fn get_file_info_by_index(&mut self, index: usize) -> Result<JsValue, JsValue> {
        let file = self.archive.by_index(index)
            .map_err(|e| JsValue::from_str(&format!("File index out of range: {:?}", e)))?;
        
        let info = FileEntryInfo {
            index,
            name: file.name().to_string(),
            uncompressed_size: file.size(),
            compressed_size: file.compressed_size(),
            crc32: file.crc32(),
            compression_method: file.compression_method() as u32,
            data_start: file.data_start(),
        };
        serde_wasm_bindgen::to_value(&info)
            .map_err(|e| JsValue::from_str(&format!("Serialization error: {:?}", e)))
    }

    /// Get all file metadata entries in the archive
    pub fn get_all_entries(&mut self) -> Result<JsValue, JsValue> {
        let count = self.archive.len();
        let mut entries = Vec::with_capacity(count);
        for i in 0..count {
            if let Ok(file) = self.archive.by_index(i) {
                entries.push(FileEntryInfo {
                    index: i,
                    name: file.name().to_string(),
                    uncompressed_size: file.size(),
                    compressed_size: file.compressed_size(),
                    crc32: file.crc32(),
                    compression_method: file.compression_method() as u32,
                    data_start: file.data_start(),
                });
            }
        }
        serde_wasm_bindgen::to_value(&entries)
            .map_err(|e| JsValue::from_str(&format!("Serialization error: {:?}", e)))
    }
}

// Stateless helper functions for quick one-off calls as specified in plan.txt

/// Stateless function: Accept TRE file bytes and return list of filenames
#[wasm_bindgen]
pub fn list_files(tre_bytes: &[u8]) -> Result<Vec<String>, JsValue> {
    let handle = TreArchiveHandle::new(tre_bytes)?;
    Ok(handle.list_files())
}

/// Stateless function: Accept TRE file bytes and filename, return extracted uncompressed bytes
#[wasm_bindgen]
pub fn extract_file(tre_bytes: &[u8], filename: &str) -> Result<js_sys::Uint8Array, JsValue> {
    let mut handle = TreArchiveHandle::new(tre_bytes)?;
    handle.extract_file(filename)
}

/// Stateless function: Return total file count from TRE bytes
#[wasm_bindgen]
pub fn get_tre_file_count(tre_bytes: &[u8]) -> Result<usize, JsValue> {
    let handle = TreArchiveHandle::new(tre_bytes)?;
    Ok(handle.file_count())
}

/// Helper function to create a demo .tre archive with synthetic SWG assets for testing
#[wasm_bindgen]
pub fn create_demo_tre() -> Result<js_sys::Uint8Array, JsValue> {
    use std::io::Write;
    use swg_tre::{write::TreWriterOptions, CompressionMethod, TreWriter};

    let mut buffer = Cursor::new(Vec::new());
    let options = TreWriterOptions::builder()
        .name_compression(CompressionMethod::Zlib)
        .record_compression(CompressionMethod::Zlib)
        .build();

    let mut writer = TreWriter::new(&mut buffer, options);

    // Real DDS texture of user's cat (compressed via Zlib into the archive)
    const CAT_DDS: &[u8] = include_bytes!("../tatooine_dune_01.dds");
    writer.start_file("texture/tatooine_dune_01.dds", CompressionMethod::Zlib)
        .map_err(|e| JsValue::from_str(&format!("{:?}", e)))?;
    writer.write_all(CAT_DDS)
        .map_err(|e| JsValue::from_str(&format!("{:?}", e)))?;

    // Simulated SHT shader file
    writer.start_file("shader/creature_womp_rat.sht", CompressionMethod::Zlib)
        .map_err(|e| JsValue::from_str(&format!("{:?}", e)))?;
    writer.write_all(b"FORM:SHDR | EFFECT: effect/default.eft | MAIN: texture/tatooine_dune_01.dds")
        .map_err(|e| JsValue::from_str(&format!("{:?}", e)))?;

    // Simulated MGN mesh file
    writer.start_file("appearance/mesh/creature_womp_rat.mgn", CompressionMethod::Zlib)
        .map_err(|e| JsValue::from_str(&format!("{:?}", e)))?;
    writer.write_all(b"FORM:MGN | VER: 0001 | INFO: verts=128, bones=14 | POSN: [0.0, 1.2, 0.4] ...")
        .map_err(|e| JsValue::from_str(&format!("{:?}", e)))?;

    // Uncompressed config manifest entry
    writer.start_file("misc/swg_archive_manifest.txt", CompressionMethod::None)
        .map_err(|e| JsValue::from_str(&format!("{:?}", e)))?;
    writer.write_all(b"Star Wars Galaxies TRE Archive Demo\nGenerated by swg-tre-wasm\n")
        .map_err(|e| JsValue::from_str(&format!("{:?}", e)))?;

    writer.finish()
        .map_err(|e| JsValue::from_str(&format!("Failed to finalize demo TRE: {:?}", e)))?;

    let bytes = buffer.into_inner();
    Ok(js_sys::Uint8Array::from(&bytes[..]))
}

