use crate::source::SourceContractError;

pub const HITOMI_METADATA_ORIGIN: &str = "https://ltn.gold-usergeneratedcontent.net";
pub const NOZOMI_CONTENT_TYPE: &str = "application/x-nozomi";

pub fn galleryinfo_script_url(gallery_id: u64) -> Result<String, SourceContractError> {
    if gallery_id == 0 {
        return Err(SourceContractError::validation(
            "galleryId",
            "must be positive",
        ));
    }
    Ok(format!(
        "{HITOMI_METADATA_ORIGIN}/galleries/{gallery_id}.js"
    ))
}

pub fn gg_script_url() -> String {
    format!("{HITOMI_METADATA_ORIGIN}/gg.js")
}

pub fn index_all_nozomi_url() -> String {
    format!("{HITOMI_METADATA_ORIGIN}/index-all.nozomi")
}

pub fn galleries_index_version_url() -> String {
    format!("{HITOMI_METADATA_ORIGIN}/galleriesindex/version")
}

pub fn galleries_index_file_url(
    version: &str,
    extension: &str,
) -> Result<String, SourceContractError> {
    let version = version.trim();
    if version.is_empty()
        || version.len() > 64
        || !version
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(SourceContractError::invalid_data(
            "galleries index version",
            "must be a non-empty safe ASCII path segment of at most 64 bytes",
        ));
    }
    if !matches!(extension, "index" | "data") {
        return Err(SourceContractError::validation(
            "galleriesIndexExtension",
            "must be index or data",
        ));
    }
    Ok(format!(
        "{HITOMI_METADATA_ORIGIN}/galleriesindex/galleries.{version}.{extension}"
    ))
}
