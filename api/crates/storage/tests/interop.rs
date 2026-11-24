//! A tree written by the older server and a tree written by this store are the
//! same tree, and each reads the other's. Run through `interop.sh`, which
//! writes the older server's tree first.

mod support;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use stellarr_storage::{FolderStore, ObjectStorage};
use support::{put, read};

const OBJECTS: &[(&str, usize, i64)] = &[
    ("images/2026/11/0f1e2d3c.png", 5000, 1),
    ("images/previews/2026/11/0f1e2d3c.webp", 300, 2),
    ("audio/2026/11/aa.webm", 70000, 3),
    ("videos/2026/11/bb.mp4", 1_000_000, 4),
    ("files/2026/11/cc.pdf", 0, 5),
    ("encrypted/2026/11/dd.bin", 10, 6),
    ("embeds/2026/11/ee.jpg", 10, 7),
    ("a/.dot/.hidden", 5, 8),
    ("żółć/ü.bin", 20, 9),
    ("with space/a b+c%d.bin", 7, 10),
];

fn bytes(length: usize, seed: i64) -> Vec<u8> {
    (0..length as i64)
        .map(|i| ((i * 31) + (seed * 7) + (i >> 8)) as u8)
        .collect()
}

/// What a tree is made of, without when it was made.
#[derive(Debug, PartialEq, Eq)]
struct Entry {
    kind: &'static str,
    size: u64,
    mode: u32,
    digest: String,
}

fn survey(root: &Path) -> BTreeMap<String, Entry> {
    use std::os::unix::fs::PermissionsExt;
    fn walk(folder: &Path, root: &Path, out: &mut BTreeMap<String, Entry>) {
        for entry in std::fs::read_dir(folder).unwrap() {
            let path = entry.unwrap().path();
            let meta = std::fs::symlink_metadata(&path).unwrap();
            let name = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .into_owned();
            if meta.is_dir() {
                out.insert(
                    name,
                    Entry {
                        kind: "folder",
                        size: 0,
                        mode: meta.permissions().mode() & 0o777,
                        digest: String::new(),
                    },
                );
                walk(&path, root, out);
            } else {
                out.insert(
                    name,
                    Entry {
                        kind: "file",
                        size: meta.len(),
                        mode: meta.permissions().mode() & 0o777,
                        digest: support::sha256_hex(&std::fs::read(&path).unwrap()),
                    },
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

fn tree(variable: &str) -> PathBuf {
    PathBuf::from(
        std::env::var(variable).unwrap_or_else(|_| panic!("{variable} is not set; run interop.sh")),
    )
}

#[tokio::test]
#[ignore = "needs the tree of the C# store: see tests/interop.sh"]
async fn the_two_stores_write_the_same_tree_and_read_each_others() {
    let older = tree("STELLARR_TEST_CS_TREE");
    let ours = tree("STELLARR_TEST_RS_TREE");

    // Write the objects with this store, and keep the tree for the other one to read.
    let store = FolderStore::new(&ours);
    store.prepare().await.unwrap();
    for (key, length, seed) in OBJECTS {
        put(&store, key, &bytes(*length, *seed)).await.unwrap();
    }

    // The same names, kinds, sizes, permissions and bytes.
    let (a, b) = (survey(&older), survey(&ours));
    let differences: Vec<String> = a
        .keys()
        .chain(b.keys())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .filter(|name| a.get(*name) != b.get(*name))
        .map(|name| format!("{name}: C# {:?} / Rust {:?}", a.get(name), b.get(name)))
        .collect();
    assert!(
        differences.is_empty(),
        "the trees differ:\n{}",
        differences.join("\n")
    );
    eprintln!(
        "interop: {} entries in each tree, identical (names, kinds, sizes, permissions, bytes)",
        a.len()
    );

    // This store reads the tree the older one wrote, and changes it.
    let reading = FolderStore::new(&older);
    let listed = reading.list().collect().await.unwrap();
    let mut expected: Vec<(String, u64)> = OBJECTS
        .iter()
        .map(|(k, l, _)| (k.to_string(), *l as u64))
        .collect();
    expected.sort_by(|x, y| x.0.as_bytes().cmp(y.0.as_bytes()));
    assert_eq!(
        listed
            .iter()
            .map(|o| (o.key.clone(), o.size))
            .collect::<Vec<_>>(),
        expected
    );
    for (key, length, seed) in OBJECTS {
        let object = reading.get(key, None).await.unwrap();
        assert_eq!(read(object).await, bytes(*length, *seed), "{key}");
    }
    reading.prepare().await.unwrap();
    reading.delete("embeds/2026/11/ee.jpg").await.unwrap();
    put(&reading, "embeds/2026/11/ff.jpg", b"new")
        .await
        .unwrap();
    assert_eq!(reading.list().collect().await.unwrap().len(), OBJECTS.len());
}
