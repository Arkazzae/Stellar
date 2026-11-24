use super::*;

#[test]
fn a_secret_segment_is_replaced_and_the_rest_is_kept() {
    assert_eq!(
        masked("/api/invites/{code}", "/api/invites/abc123"),
        "/api/invites/***"
    );
    assert_eq!(
        masked("/api/invites/{code}/join", "/api/invites/abc123/join"),
        "/api/invites/***/join"
    );
    assert_eq!(
        masked("/api/channels/{id}", "/api/channels/x"),
        "/api/channels/x"
    );
    assert_eq!(
        masked("/api/invites/{code}", "/api/invites/"),
        "/api/invites/***"
    );
}

#[test]
fn every_parameter_of_every_route_is_an_id_or_a_secret_and_a_secret_is_masked() {
    let mut all = crate::http::Endpoints::new();
    for group in crate::groups() {
        all = all.merge(group);
    }
    let (_, rules) = all.into_parts();
    let mut unknown = std::collections::BTreeSet::new();
    let mut used = std::collections::BTreeSet::new();
    let mut secret_routes = std::collections::BTreeSet::new();
    for (_, pattern) in rules.keys() {
        for name in parameters(pattern) {
            used.insert(name);
            if SECRETS.contains(&name) {
                secret_routes.insert(pattern.clone());
            } else if !OPEN.contains(&name) {
                unknown.insert(format!("{pattern}: {name}"));
            }
        }
    }
    assert!(
        unknown.is_empty(),
        "a route has a parameter that is neither an id nor a secret (see http/secret_path.rs): {unknown:?}"
    );
    // A name nobody uses would let a future route with it pass undecided.
    let idle: Vec<&&str> = OPEN
        .iter()
        .chain(SECRETS)
        .filter(|name| !used.contains(**name))
        .collect();
    assert!(
        idle.is_empty(),
        "no route has a parameter called {idle:?}: take it off the list"
    );
    let secret_routes: Vec<String> = secret_routes.into_iter().collect();
    assert_eq!(
        secret_routes
            .iter()
            .filter(|route| route.contains("{code}"))
            .collect::<Vec<_>>(),
        ["/api/invites/{code}", "/api/invites/{code}/join"]
    );
    assert!(
        secret_routes
            .iter()
            .any(|route| route.contains("/desktop/browser/verify/{request_id}")),
        "{secret_routes:?}"
    );
}

#[test]
fn a_path_no_route_took_shows_only_the_words_the_routes_themselves_use() {
    let words = words(
        [
            "/api/invites/{code}",
            "/api/invites/{code}/join",
            "/api/channels/{id}",
        ]
        .into_iter(),
    );
    for (path, shown) in [
        ("/api/invites//SECRET", "/api/invites//***"),
        ("/api/invites/SECRET/extra", "/api/invites/***/***"),
        ("/API/INVITES/SECRET", "/***/***/***"),
        ("/api/Invites/SECRET", "/api/***/***"),
        ("/api/invites%2FSECRET", "/api/***"),
        ("/api/invites/SECRET/JOIN", "/api/invites/***/***"),
        ("/invite/SECRET", "/***/***"),
        ("/api/channels/x", "/api/channels/***"),
        ("/", "/"),
    ] {
        assert_eq!(masked_unmatched(&words, path), shown, "{path}");
    }
}
