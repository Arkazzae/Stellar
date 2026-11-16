//! Users and groups by name, read from /etc/passwd and /etc/group.
//!
//! The container's accounts all live in those two files, so reading them
//! directly gives the same answer as the C library without loading its name
//! service modules into PID 1.

use std::fs;

/// The user and group a service runs as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Account {
    pub uid: u32,
    pub gid: u32,
}

/// Looks up `user`, and `group` when given; otherwise the user's own group.
pub fn resolve(user: &str, group: Option<&str>) -> Result<Account, String> {
    let passwd =
        fs::read_to_string("/etc/passwd").map_err(|e| format!("cannot read /etc/passwd: {e}"))?;
    let (uid, primary) = find_user(&passwd, user)
        .ok_or_else(|| format!("there is no user {user} in /etc/passwd"))?;
    let gid = match group {
        None => primary,
        Some(group) => {
            let groups = fs::read_to_string("/etc/group")
                .map_err(|e| format!("cannot read /etc/group: {e}"))?;
            find_group(&groups, group)
                .ok_or_else(|| format!("there is no group {group} in /etc/group"))?
        }
    };
    Ok(Account { uid, gid })
}

/// The user ID and primary group ID of `name` in a passwd file.
pub fn find_user(passwd: &str, name: &str) -> Option<(u32, u32)> {
    passwd.lines().find_map(|line| {
        let mut fields = line.split(':');
        if fields.next()? != name {
            return None;
        }
        let _password = fields.next()?;
        let uid = fields.next()?.parse().ok()?;
        let gid = fields.next()?.parse().ok()?;
        Some((uid, gid))
    })
}

/// The group ID of `name` in a group file.
pub fn find_group(groups: &str, name: &str) -> Option<u32> {
    groups.lines().find_map(|line| {
        let mut fields = line.split(':');
        if fields.next()? != name {
            return None;
        }
        let _password = fields.next()?;
        fields.next()?.parse().ok()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const PASSWD: &str = "root:x:0:0:root:/root:/bin/bash\n\
        www-data:x:33:33:www-data:/var/www:/usr/sbin/nologin\n\
        nobody:x:65534:65534:nobody:/nonexistent:/usr/sbin/nologin\n\
        broken:x:abc:1::/:/bin/false\n";

    const GROUP: &str = "root:x:0:\nwww-data:x:33:\nnogroup:x:65534:\n";

    #[test]
    fn users_and_groups_are_found_by_exact_name() {
        assert_eq!(find_user(PASSWD, "nobody"), Some((65534, 65534)));
        assert_eq!(find_user(PASSWD, "www-data"), Some((33, 33)));
        assert_eq!(find_user(PASSWD, "nob"), None);
        assert_eq!(find_user(PASSWD, "broken"), None);
        assert_eq!(find_user("", "root"), None);
        assert_eq!(find_group(GROUP, "nogroup"), Some(65534));
        assert_eq!(find_group(GROUP, "nobody"), None);
    }
}
