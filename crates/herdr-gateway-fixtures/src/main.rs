use std::path::PathBuf;

fn main() {
    let dir = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/herdr-gateway-v1")
        });
    match herdr_gateway_fixtures::validate_corpus(&dir) {
        Ok(count) => println!("validated {count} Herdr gateway v1 fixtures"),
        Err(error) => {
            eprintln!("herdr gateway fixtures: {error}");
            std::process::exit(1);
        }
    }
}
