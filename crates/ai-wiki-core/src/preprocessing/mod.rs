pub mod detect;
pub mod pdf;
pub mod zip_extract;
pub mod media;

pub use detect::{detect_file_type, FileClassification};
pub use pdf::{classify_pdf, split_pdf_chapters, extract_pdf_text};
pub use zip_extract::extract_zip;
pub use media::{extract_audio, transcribe_audio};
