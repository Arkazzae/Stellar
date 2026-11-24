// The version of the server is read at compile time from `STELLARR_VERSION`
// (see `src/version.rs`); cargo has to be told to build again when it changes.
fn main() {
    println!("cargo:rerun-if-env-changed=STELLARR_VERSION");
}
