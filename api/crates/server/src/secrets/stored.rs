//! Encrypted JSON records in the instance database, bound to their name and purpose.
use crate::seal::Sealer;
use crate::values::Timestamp;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

pub(crate) fn read<T: for<'de> Deserialize<'de>>(
    connection: &Connection,
    sealer: &Sealer,
    row: &str,
    purpose: &str,
) -> rusqlite::Result<Option<T>> {
    let Some(sealed) = connection
        .query_row(
            "SELECT \"ProtectedValue\" FROM \"InstanceSecrets\" WHERE \"Name\" = ?1",
            [row],
            |found| found.get::<_, String>(0),
        )
        .optional()?
    else {
        return Ok(None);
    };
    let bytes = sealer.open(purpose, row.as_bytes(), &sealed, Timestamp::now())?;
    serde_json::from_slice::<T>(&bytes)
        .map(Some)
        .map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })
}

pub(crate) fn write<T: Serialize>(
    connection: &Connection,
    sealer: &Sealer,
    row: &str,
    purpose: &str,
    value: &T,
) -> rusqlite::Result<()> {
    let payload = serde_json::to_vec(value)
        .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
    let sealed = sealer.seal(purpose, row.as_bytes(), None, &payload);
    connection.execute(
        "INSERT INTO \"InstanceSecrets\" (\"Name\", \"ProtectedValue\", \"UpdatedAt\") VALUES (?1, ?2, ?3)
         ON CONFLICT (\"Name\") DO UPDATE SET \"ProtectedValue\" = excluded.\"ProtectedValue\", \"UpdatedAt\" = excluded.\"UpdatedAt\"",
        params![row, sealed, Timestamp::now()],
    )?;
    Ok(())
}
