// Compile the API section of each language pack into static strings. The English
// keys are the enum used at call sites, so renaming a message checks its callers.
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fmt::Write;
use std::fs;
use std::path::Path;

type Catalog = BTreeMap<String, String>;

fn placeholders(text: &str) -> BTreeSet<&str> {
    text.split('{')
        .skip(1)
        .filter_map(|part| part.split_once('}').map(|(name, _)| name))
        .collect()
}

fn language_variant(code: &str) -> String {
    // Keep the public variant names used by the server and mail crates.
    match code {
        "en" => "English".into(),
        "pl" => "Polish".into(),
        _ => {
            let mut variant = String::from("Lang");
            for part in code.split('-') {
                assert!(
                    !part.is_empty() && part.bytes().all(|b| b.is_ascii_alphanumeric()),
                    "invalid locale code {code:?}"
                );
                variant.push_str(&part[..1].to_ascii_uppercase());
                variant.push_str(&part[1..]);
            }
            variant
        }
    }
}

fn main() {
    let directory = Path::new("../../../locales");
    println!("cargo:rerun-if-changed={}", directory.display());
    let mut catalogs = BTreeMap::new();
    for entry in fs::read_dir(directory).expect("read locales/") {
        let path = entry.unwrap().path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let code = path.file_stem().unwrap().to_str().unwrap().to_owned();
        let mut pack: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap())
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        let mut api: Catalog = serde_json::from_value(pack["api"].take())
            .unwrap_or_else(|error| panic!("{}.api: {error}", path.display()));
        api.retain(|key, _| !key.starts_with("_context."));
        catalogs.insert(code, api);
    }
    let english = &catalogs["en"];
    let keys: Vec<_> = english.keys().map(String::as_str).collect();
    let variants: Vec<_> = catalogs.keys().map(|code| language_variant(code)).collect();

    let mut out = String::new();
    for (name, names) in [
        ("MessageKey", keys.clone()),
        ("Language", variants.iter().map(String::as_str).collect()),
    ] {
        writeln!(
            out,
            "#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)] pub enum {name} {{ {} }}",
            names.join(",")
        )
        .unwrap();
        let values = names
            .iter()
            .map(|key| format!("Self::{key}"))
            .collect::<Vec<_>>()
            .join(",");
        writeln!(
            out,
            "impl {name} {{ pub const ALL: &'static [Self] = &[{values}]; }}"
        )
        .unwrap();
    }

    out.push_str("const CATALOGS: &[(&str, &[&str])] = &[\n");
    for (code, api) in &catalogs {
        for key in api.keys() {
            assert!(
                english.contains_key(key),
                "{code}.api.{key} is absent from en.json"
            );
        }
        writeln!(out, "({code:?}, &[").unwrap();
        for key in &keys {
            assert!(
                key.starts_with(|c: char| c.is_ascii_uppercase())
                    && key.bytes().all(|b| b.is_ascii_alphanumeric()),
                "invalid API key {key:?}"
            );
            let text = api
                .get(*key)
                .unwrap_or_else(|| panic!("{code}.api.{key} is missing"));
            assert!(!text.trim().is_empty(), "{code}.api.{key} is empty");
            assert_eq!(
                placeholders(text),
                placeholders(&english[*key]),
                "{code}.api.{key} placeholders differ from English"
            );
            writeln!(out, "{text:?},").unwrap();
        }
        out.push_str("]),\n");
    }
    out.push_str("];\n");
    fs::write(
        Path::new(&env::var_os("OUT_DIR").unwrap()).join("catalogs.rs"),
        out,
    )
    .unwrap();
}
