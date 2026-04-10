pub mod detect;
pub mod media;
pub mod pdf;
pub mod zip_extract;

pub use detect::{FileClassification, detect_file_type};
pub use media::{extract_audio, transcribe_audio};
pub use pdf::{classify_pdf, extract_pdf_text, split_pdf_chapters};
pub use zip_extract::extract_zip;
