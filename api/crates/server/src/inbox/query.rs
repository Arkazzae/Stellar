//! The query of the inbox's list: `limit` and `beforeSequence`, bound as the
//! first server bound them, by the shared reader of query values.

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
