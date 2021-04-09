//! Download compiler dependency artifacts.

use std::path::Path;

use flate2::read::GzDecoder;
use reqwest::Client;
use tar::Archive;

static APP_USER_AGENT: &str = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"),);

#[tokio::main]
async fn main() {
    std::fs::remove_dir_all(Path::new("../../compiler-deps")).unwrap();
    std::fs::create_dir_all(Path::new("../../compiler-deps")).unwrap();

    for asset in &["dev-linux", "dev-win", "target-linux", "target-win"] {
        // Download this asset.
        let asset_downloaded = Client::builder()
            .user_agent(APP_USER_AGENT)
            .build()
            .unwrap()
            .get(format!(
                "https://github.com/quill-lang/compiler-deps/releases/download/latest/{}.tar.gz",
                asset
            ))
            .send()
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap();

        let decoder = GzDecoder::new(&*asset_downloaded);
        let mut archive = Archive::new(decoder);
        archive.unpack(Path::new("../../compiler-deps")).unwrap();
    }
}