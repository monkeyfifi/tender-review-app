mod docx;
mod pdf;
pub mod preflight;

pub use docx::DocxParser;
pub use pdf::PdfParser;

use crate::{domain::document::NormalizedDocument, error::AppError};
use std::{fs::File, io::Read, path::Path};

pub(crate) const MAX_DOCUMENT_FILE_BYTES: u64 = 100 * 1024 * 1024;

#[derive(Debug)]
pub(crate) struct BoundedFileSnapshot {
    pub bytes: Vec<u8>,
    pub byte_size: u64,
}

pub(crate) fn read_file_bounded(
    path: &Path,
    max_bytes: u64,
    subject: &str,
) -> Result<BoundedFileSnapshot, AppError> {
    let file = File::open(path).map_err(|_| AppError::unreadable_document("无法打开文件"))?;
    let metadata = file
        .metadata()
        .map_err(|_| AppError::unreadable_document("无法读取文件元数据"))?;
    if !metadata.is_file() || metadata.len() == 0 {
        return Err(AppError::invalid_file_selection());
    }
    if metadata.len() > max_bytes {
        return Err(file_too_large(subject, max_bytes));
    }

    let bytes = read_stream_bounded(file, max_bytes, subject)?;
    Ok(BoundedFileSnapshot {
        byte_size: bytes.len() as u64,
        bytes,
    })
}

pub(crate) fn read_stream_bounded<R: Read>(
    reader: R,
    max_bytes: u64,
    subject: &str,
) -> Result<Vec<u8>, AppError> {
    let mut reader = reader.take(max_bytes.saturating_add(1));
    let mut bytes = Vec::new();
    reader
        .read_to_end(&mut bytes)
        .map_err(|_| AppError::unreadable_document("无法读取文件内容"))?;
    if bytes.len() as u64 > max_bytes {
        return Err(file_too_large(subject, max_bytes));
    }
    Ok(bytes)
}

fn file_too_large(subject: &str, max_bytes: u64) -> AppError {
    AppError::unreadable_document(format!(
        "{subject}大小超过 {} MiB 限制",
        max_bytes / (1024 * 1024)
    ))
}

pub trait DocumentParser {
    fn parse(&self, path: &Path, source_label: &str) -> Result<NormalizedDocument, AppError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCode;
    use std::{
        cell::Cell,
        io::{Cursor, Read},
        rc::Rc,
    };

    struct TrackingReader {
        inner: Cursor<Vec<u8>>,
        bytes_read: Rc<Cell<usize>>,
    }

    impl Read for TrackingReader {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            let read = self.inner.read(buffer)?;
            self.bytes_read.set(self.bytes_read.get() + read);
            Ok(read)
        }
    }

    #[test]
    fn bounded_stream_stops_after_limit_plus_one_byte() {
        let bytes_read = Rc::new(Cell::new(0));
        let reader = TrackingReader {
            inner: Cursor::new(vec![0_u8; 64]),
            bytes_read: bytes_read.clone(),
        };

        let error = read_stream_bounded(reader, 3, "测试文件").unwrap_err();

        assert_eq!(error.code, ErrorCode::UnreadableDocument);
        assert_eq!(bytes_read.get(), 4);
        assert!(error.message.contains("测试文件"));
    }

    #[test]
    fn bounded_file_rejects_oversized_metadata_without_leaking_path() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("sensitive-filename.docx");
        let file = std::fs::File::create(&path).unwrap();
        file.set_len(5).unwrap();

        let error = read_file_bounded(&path, 4, "测试文件").unwrap_err();

        assert_eq!(error.code, ErrorCode::UnreadableDocument);
        assert!(!error.message.contains(&path.display().to_string()));
    }
}
