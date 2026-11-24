//! The groups a connection belongs to, and how a person writes an id the hub
//! is asked about.
//!
//! No group stands for who may see something: a group's members would go
//! stale when roles change. There is one per account, one per server and one
//! per account on the server a connection follows, and whoever may see an
//! event is worked out when it is sent.

use crate::values::Id;

/// An id as the group names write it: 32 hexadecimal digits, no hyphens.
fn compact(id: Id) -> String {
    id.to_string().replace('-', "")
}

/// Every connection that follows the server.
pub fn server_group(server: Id) -> String {
    format!("server:{}", compact(server))
}

/// The connections of one account that follow one server: what an event
/// meant for some of a server's members goes to. Unlike the account's own
/// group it leaves out the account's tabs on other servers, which would take a
/// message for a channel they do not know as something to sound.
pub fn follower_group(server: Id, user: Id) -> String {
    format!("server:{}:user:{}", compact(server), compact(user))
}

/// Every connection of one account.
pub fn user_group(user: Id) -> String {
    format!("user:{}", compact(user))
}

fn hex(byte: u8) -> Option<u8> {
    char::from(byte)
        .to_digit(16)
        .and_then(|digit| u8::try_from(digit).ok())
}

/// The text after `name=` in a query string, read as a browser sent it:
/// percent-decoded, with `+` for a space, and found without regard to the
/// case of the name. The first one wins.
pub fn query_value(query: &str, name: &str) -> Option<String> {
    query.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        (decode(key).eq_ignore_ascii_case(name)).then(|| decode(value))
    })
}

fn decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        match bytes[at] {
            b'+' => out.push(b' '),
            b'%' if at + 2 < bytes.len() => match (hex(bytes[at + 1]), hex(bytes[at + 2])) {
                (Some(high), Some(low)) => {
                    out.push(high << 4 | low);
                    at += 2;
                }
                _ => out.push(b'%'),
            },
            byte => out.push(byte),
        }
        at += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "3d82da44-8aa5-4bdf-9ee1-2902df99880c";

    #[test]
    fn the_groups_are_named_with_ids_without_hyphens() {
        let server = Id::parse(ID).unwrap();
        let user = Id::parse("30000000-0000-0000-0000-000000000001").unwrap();

        assert_eq!(
            server_group(server),
            "server:3d82da448aa54bdf9ee12902df99880c"
        );
        assert_eq!(user_group(user), "user:30000000000000000000000000000001");
        assert_eq!(
            follower_group(server, user),
            "server:3d82da448aa54bdf9ee12902df99880c:user:30000000000000000000000000000001"
        );
    }

    #[test]
    fn a_query_value_is_found_by_any_case_of_its_name_and_decoded() {
        assert_eq!(
            query_value("serverId=abc&x=1", "serverId").as_deref(),
            Some("abc")
        );
        assert_eq!(
            query_value("x=1&SERVERID=abc", "serverId").as_deref(),
            Some("abc")
        );
        assert_eq!(
            query_value("serverId=a%2Db+c", "serverId").as_deref(),
            Some("a-b c")
        );
        assert_eq!(
            query_value("serverId=first&serverId=second", "serverId").as_deref(),
            Some("first")
        );
        assert_eq!(query_value("serverId", "serverId").as_deref(), Some(""));
        assert_eq!(query_value("other=1", "serverId"), None);
        assert_eq!(
            query_value("serverId=%zz%4", "serverId").as_deref(),
            Some("%zz%4")
        );
    }
}
