/// Declares an enum that the database stores as an integer.
///
/// ```ignore
/// int_enum! {
///     /// Whether a channel carries text or voice.
///     #[derive(serde::Serialize, serde::Deserialize)]
///     #[serde(rename_all = "lowercase")]
///     pub enum ChannelKind { Text = 0, Voice = 1 }
/// }
/// ```
///
/// The enum gets `to_int`, `from_int` and the `rusqlite` conversions. How it
/// looks in JSON is the business of the `serde` attributes next to it, since
/// that differs from one enum to the next.
#[macro_export]
macro_rules! int_enum {
    (
        $(#[$meta:meta])*
        $vis:vis enum $name:ident { $($variant:ident = $value:literal),+ $(,)? }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        $vis enum $name { $($variant),+ }

        impl $name {
            pub fn to_int(self) -> i64 {
                match self { $($name::$variant => $value),+ }
            }

            pub fn from_int(value: i64) -> Option<$name> {
                match value { $($value => Some($name::$variant),)+ _ => None }
            }
        }

        impl rusqlite::types::ToSql for $name {
            fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {
                Ok(rusqlite::types::ToSqlOutput::from(self.to_int()))
            }
        }

        impl rusqlite::types::FromSql for $name {
            fn column_result(
                value: rusqlite::types::ValueRef<'_>,
            ) -> rusqlite::types::FromSqlResult<$name> {
                let number = value.as_i64()?;
                $name::from_int(number).ok_or(rusqlite::types::FromSqlError::OutOfRange(number))
            }
        }
    };
}

#[cfg(test)]
mod tests {
    int_enum! {
        #[derive(serde::Serialize)]
        #[serde(rename_all = "lowercase")]
        enum ChannelKind { Text = 0, Voice = 1 }
    }

    // The database stores `general` as kind 0 and `voice` as kind 1.
    #[test]
    fn an_enum_is_the_integer_the_database_holds() {
        let db = rusqlite::Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE Channels (Name TEXT, Kind INTEGER NOT NULL);
             INSERT INTO Channels VALUES ('general', 0), ('voice', 1), ('broken', 7);",
        )
        .unwrap();

        let kind = |name: &str| -> rusqlite::Result<ChannelKind> {
            db.query_row("SELECT Kind FROM Channels WHERE Name = ?1", [name], |row| {
                row.get(0)
            })
        };

        assert_eq!(kind("general").unwrap(), ChannelKind::Text);
        assert_eq!(kind("voice").unwrap(), ChannelKind::Voice);
        assert!(kind("broken").is_err());
        db.execute(
            "UPDATE Channels SET Kind = ?1 WHERE Name = 'general'",
            [ChannelKind::Voice],
        )
        .unwrap();
        assert_eq!(kind("general").unwrap(), ChannelKind::Voice);
        assert_eq!(
            serde_json::to_string(&ChannelKind::Voice).unwrap(),
            "\"voice\""
        );
    }
}
