use sha2::{Digest, Sha256};

use crate::source::SourceContractError;

use super::parse_nozomi_ids;

pub const GALLERIES_INDEX_NODE_BYTES: u64 = 464;
pub const GALLERIES_INDEX_MAX_DEPTH: usize = 64;
const GALLERIES_INDEX_MAX_KEYS: usize = 16;
const GALLERIES_INDEX_CHILDREN: usize = GALLERIES_INDEX_MAX_KEYS + 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GalleryIndexDataRange {
    pub offset: u64,
    pub length: u32,
}

impl GalleryIndexDataRange {
    pub fn header_value(self) -> Result<String, SourceContractError> {
        if self.length < 4 || !self.length.is_multiple_of(4) {
            return Err(SourceContractError::invalid_data(
                "galleries index data range",
                format!(
                    "length must contain a 4-byte count followed by gallery IDs, got {}",
                    self.length
                ),
            ));
        }
        let end_inclusive = self
            .offset
            .checked_add(u64::from(self.length))
            .and_then(|end_exclusive| end_exclusive.checked_sub(1))
            .ok_or_else(|| {
                SourceContractError::invalid_data(
                    "galleries index data range",
                    "byte range overflows u64",
                )
            })?;
        Ok(format!("bytes={}-{}", self.offset, end_inclusive))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GalleryIndexNode {
    keys: Vec<Vec<u8>>,
    data: Vec<GalleryIndexDataRange>,
    children: Vec<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GalleryIndexLookup {
    Match(GalleryIndexDataRange),
    Child(u64),
    Missing,
}

impl GalleryIndexNode {
    pub fn lookup(&self, key: &[u8]) -> Result<GalleryIndexLookup, SourceContractError> {
        let slot = self
            .keys
            .partition_point(|candidate| candidate.as_slice() < key);
        if self
            .keys
            .get(slot)
            .is_some_and(|candidate| candidate.as_slice() == key)
        {
            return Ok(GalleryIndexLookup::Match(self.data[slot]));
        }
        if self.children.iter().all(|address| *address == 0) {
            return Ok(GalleryIndexLookup::Missing);
        }
        let address = self.children.get(slot).copied().unwrap_or_default();
        if address == 0 {
            return Err(SourceContractError::invalid_data(
                "galleries index node",
                format!("internal node has no child at search slot {slot}"),
            ));
        }
        Ok(GalleryIndexLookup::Child(address))
    }
}

pub fn gallery_index_term_key(term: &str) -> [u8; 4] {
    let digest = Sha256::digest(term.as_bytes());
    digest[..4]
        .try_into()
        .expect("SHA-256 always has four bytes")
}

pub fn parse_gallery_index_version(bytes: &[u8]) -> Result<String, SourceContractError> {
    let version = std::str::from_utf8(bytes)
        .map_err(|error| {
            SourceContractError::invalid_data("galleries index version", error.to_string())
        })?
        .trim();
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
    Ok(version.to_owned())
}

pub fn parse_gallery_index_node(bytes: &[u8]) -> Result<GalleryIndexNode, SourceContractError> {
    let mut cursor = 0;
    let key_count = read_u32(bytes, &mut cursor, "key count")? as usize;
    if key_count > GALLERIES_INDEX_MAX_KEYS {
        return Err(SourceContractError::invalid_data(
            "galleries index node",
            format!("key count must not exceed {GALLERIES_INDEX_MAX_KEYS}, got {key_count}"),
        ));
    }

    let mut keys = Vec::with_capacity(key_count);
    for index in 0..key_count {
        let length = read_u32(bytes, &mut cursor, "key length")? as usize;
        if !(1..=32).contains(&length) {
            return Err(SourceContractError::invalid_data(
                format!("galleries index key {index}"),
                format!("length must be between 1 and 32 bytes, got {length}"),
            ));
        }
        let end = cursor.checked_add(length).ok_or_else(|| {
            SourceContractError::invalid_data(
                format!("galleries index key {index}"),
                "byte offset overflows usize",
            )
        })?;
        let key = bytes.get(cursor..end).ok_or_else(|| {
            SourceContractError::invalid_data(
                format!("galleries index key {index}"),
                "payload is truncated",
            )
        })?;
        keys.push(key.to_vec());
        cursor = end;
    }
    if keys.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(SourceContractError::invalid_data(
            "galleries index node",
            "keys must be strictly increasing",
        ));
    }

    let data_count = read_u32(bytes, &mut cursor, "data count")? as usize;
    if data_count != key_count {
        return Err(SourceContractError::invalid_data(
            "galleries index node",
            format!("data count {data_count} does not match key count {key_count}"),
        ));
    }
    let mut data = Vec::with_capacity(data_count);
    for index in 0..data_count {
        let offset = read_u64(bytes, &mut cursor, "data offset")?;
        let length = read_u32(bytes, &mut cursor, "data length")?;
        let range = GalleryIndexDataRange { offset, length };
        range.header_value().map_err(|error| {
            SourceContractError::invalid_data(
                format!("galleries index data {index}"),
                error.message,
            )
        })?;
        data.push(range);
    }

    let mut children = Vec::with_capacity(GALLERIES_INDEX_CHILDREN);
    for _ in 0..GALLERIES_INDEX_CHILDREN {
        children.push(read_u64(bytes, &mut cursor, "child address")?);
    }

    Ok(GalleryIndexNode {
        keys,
        data,
        children,
    })
}

pub fn parse_gallery_index_data(bytes: &[u8]) -> Result<Vec<u64>, SourceContractError> {
    if bytes.len() < 4 {
        return Err(SourceContractError::invalid_data(
            "galleries index data",
            "payload is missing the gallery count",
        ));
    }
    let count = u32::from_be_bytes(bytes[..4].try_into().expect("length checked")) as usize;
    let expected = count
        .checked_mul(4)
        .and_then(|ids| ids.checked_add(4))
        .ok_or_else(|| {
            SourceContractError::invalid_data(
                "galleries index data",
                "declared gallery count overflows usize",
            )
        })?;
    if bytes.len() != expected {
        return Err(SourceContractError::invalid_data(
            "galleries index data",
            format!(
                "declared {count} gallery IDs require {expected} bytes, got {}",
                bytes.len()
            ),
        ));
    }
    parse_nozomi_ids(&bytes[4..])
}

fn read_u32(bytes: &[u8], cursor: &mut usize, field: &str) -> Result<u32, SourceContractError> {
    let end = cursor.checked_add(4).ok_or_else(|| {
        SourceContractError::invalid_data("galleries index node", "byte offset overflows usize")
    })?;
    let value = bytes.get(*cursor..end).ok_or_else(|| {
        SourceContractError::invalid_data(
            "galleries index node",
            format!("payload is truncated while reading {field}"),
        )
    })?;
    *cursor = end;
    Ok(u32::from_be_bytes(
        value.try_into().expect("slice length checked"),
    ))
}

fn read_u64(bytes: &[u8], cursor: &mut usize, field: &str) -> Result<u64, SourceContractError> {
    let end = cursor.checked_add(8).ok_or_else(|| {
        SourceContractError::invalid_data("galleries index node", "byte offset overflows usize")
    })?;
    let value = bytes.get(*cursor..end).ok_or_else(|| {
        SourceContractError::invalid_data(
            "galleries index node",
            format!("payload is truncated while reading {field}"),
        )
    })?;
    *cursor = end;
    Ok(u64::from_be_bytes(
        value.try_into().expect("slice length checked"),
    ))
}
