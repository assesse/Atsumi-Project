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
