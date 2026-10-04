//! Print the agent language reference: `cargo run -p cut-lang --example reference`.

fn main() {
    print!("{}", cut_lang::reference(cut_model::Catalog::builtin()));
}
