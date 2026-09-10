mod endpoints;
mod gallery_index;
mod galleryinfo;
mod model;
mod nozomi;
mod routing;
mod tag_catalog;

/// Increment when a parser change alters the typed meaning of saved source fixtures.
pub const HITOMI_PARSER_VERSION: u32 = 1;
/// Increment when routing or candidate ordering changes for the same source metadata.
pub const HITOMI_RESOLVER_VERSION: u32 = 1;

pub use endpoints::{
    galleries_index_file_url, galleries_index_version_url, galleryinfo_script_url, gg_script_url,
    index_all_nozomi_url, HITOMI_METADATA_ORIGIN, NOZOMI_CONTENT_TYPE,
};
pub(crate) use gallery_index::{
    gallery_index_term_key, parse_gallery_index_data, parse_gallery_index_node,
    parse_gallery_index_version, GalleryIndexLookup, GalleryIndexNode, GALLERIES_INDEX_MAX_DEPTH,
    GALLERIES_INDEX_NODE_BYTES,
};
pub use galleryinfo::parse_galleryinfo_script;
pub use model::{
    HitomiGalleryDetail, HitomiGalleryMetadata, HitomiGallerySummary, HitomiPageFile, HitomiTag,
    HitomiTagKind, SourceRevision, HITOMI_CONTENT_DOMAIN, HITOMI_ORIGIN,
};
pub use nozomi::{parse_nozomi_ids, parse_nozomi_range, NozomiByteRange, MAX_NOZOMI_RANGE_ITEMS};
pub use routing::{
    download_full_candidates, parse_gg_routing, webp_full_candidates, webp_thumbnail_candidates,
    GgRoutingTable, HitomiImageCandidate, HitomiImageFormat, HitomiImageKind, ThumbnailSize,
};
pub use tag_catalog::{
    all_catalog_pages, all_tags_urls, merge_catalog, parse_all_tags_page, parse_catalog_page,
    ALL_CATALOG_PAGE_COUNT, ALL_TAGS_PAGE_COUNT,
};

#[cfg(test)]
mod tests;
