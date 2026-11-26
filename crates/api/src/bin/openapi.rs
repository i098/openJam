//! Write the contract, or check it has not drifted: `openjam-openapi [--check] [path]`.

use std::path::PathBuf;

fn main() {
    let mut args = std::env::args().skip(1);
    let check = args.next().is_some_and(|a| a == "--check");
    let path = PathBuf::from(
        args.next()
            .unwrap_or_else(|| "crates/api/contract/openapi.yaml".into()),
    );
    let generated = openjam_api::openapi_yaml();
    if check {
        let current = std::fs::read_to_string(&path).unwrap_or_default();
        if current != generated {
            eprintln!(
                "{} is out of date. Run: cargo run -p openjam-api --bin openjam-openapi",
                path.display()
            );
            std::process::exit(1);
        }
        println!("{} is current", path.display());
    } else {
        std::fs::write(&path, generated).expect("write contract");
        println!("wrote {}", path.display());
    }
}
