use serde::{Deserialize, Serialize};

use crate::source::SourceContractError;

pub const MAX_NOZOMI_RANGE_ITEMS: u32 = 400;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NozomiByteRange {
    pub start: u64,
    pub end_inclusive: u64,
    pub item_count: u32,
}

impl NozomiByteRange {
    pub fn new(start_item: u64, item_count: u32) -> Result<Self, SourceContractError> {
        if item_count == 0 {
            return Err(SourceContractError::validation(
                "itemCount",
                "must be greater than zero",
            ));
        }
        if item_count > MAX_NOZOMI_RANGE_ITEMS {
            return Err(SourceContractError::validation(
                "itemCount",
                format!("must not exceed {MAX_NOZOMI_RANGE_ITEMS}"),
            ));
        }
        let start = start_item.checked_mul(4).ok_or_else(|| {
            SourceContractError::validation("startItem", "byte offset overflows u64")
        })?;
        let byte_count = u64::from(item_count).checked_mul(4).ok_or_else(|| {
            SourceContractError::validation("itemCount", "byte count overflows u64")
        })?;
        let end_inclusive = start
            .checked_add(byte_count)
            .and_then(|end_exclusive| end_exclusive.checked_sub(1))
            .ok_or_else(|| {
                SourceContractError::validation("startItem", "byte range overflows u64")
            })?;
        Ok(Self {
            start,
            end_inclusive,
            item_count,
        })
    }

    pub fn header_value(self) -> String {
        format!("bytes={}-{}", self.start, self.end_inclusive)
    }

    pub const fn expected_byte_len(self) -> usize {
        self.item_count as usize * 4
    }
}

pub fn parse_nozomi_ids(bytes: &[u8]) -> Result<Vec<u64>, SourceContractError> {
    if !bytes.len().is_multiple_of(4) {
        return Err(SourceContractError::invalid_data(
            "nozomi payload",
            format!(
                "byte length must be divisible by 4, got {} bytes",
                bytes.len()
            ),
        ));
    }

    let mut ids = Vec::with_capacity(bytes.len() / 4);
    for (index, chunk) in bytes.as_chunks::<4>().0.iter().enumerate() {
        let id = u32::from_be_bytes(*chunk);
        if id == 0 {
            return Err(SourceContractError::invalid_data(
                format!("nozomi item {index}"),
                "gallery ID must be positive",
            ));
        }
        ids.push(u64::from(id));
    }
    Ok(ids)
}

pub fn parse_nozomi_range(
    bytes: &[u8],
    requested: NozomiByteRange,
) -> Result<Vec<u64>, SourceContractError> {
    if bytes.len() != requested.expected_byte_len() {
        return Err(SourceContractError::invalid_data(
            "nozomi range payload",
            format!(
                "expected {} bytes for {} items, got {}",
                requested.expected_byte_len(),
                requested.item_count,
                bytes.len()
            ),
        ));
    }
    parse_nozomi_ids(bytes)
}
