//! Query parameters for listing the inbox: `limit` and `beforeSequence`.

use crate::http::{ApiError, QueryValues};

pub struct PageQuery {
    pub limit: Option<i32>,
    pub before_sequence: Option<i64>,
}

impl PageQuery {
    /// A value that is no number does not bind.
    pub fn read(values: &QueryValues) -> Result<PageQuery, ApiError> {
        Ok(PageQuery {
            limit: values.int("limit")?,
            before_sequence: values.long("beforeSequence")?,
        })
    }
}
