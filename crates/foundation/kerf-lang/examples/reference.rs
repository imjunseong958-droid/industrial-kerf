//! Print the agent language reference: `cargo run -p kerf-lang --example reference`.

fn main() {
    print!("{}", kerf_lang::reference(kerf_model::Catalog::builtin()));
}
