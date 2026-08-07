use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
pub struct UploadUrls {
    pub log_file_url: String,
    pub output_zip_url: String,
}
