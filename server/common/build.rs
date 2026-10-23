// sqlx::migrate! embeds the migrations at compile time: rebuild when one is added
// or changed, or a fresh binary would ship without it.
fn main() {
    println!("cargo:rerun-if-changed=../migrations");
}
