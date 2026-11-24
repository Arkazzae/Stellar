//! The groups a connection belongs to.
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
}
