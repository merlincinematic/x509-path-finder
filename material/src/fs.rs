use std::io;
use std::io::ErrorKind;
use std::path::Path;
use tokio::fs::File;
use tokio::io::AsyncReadExt;
use url::Url;
use x509_client::provided::default::DefaultX509Iterator;
use x509_client::X509ClientConfiguration;
pub async fn load_material(file: &str) -> io::Result<Vec<u8>> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(file);
    let mut file = File::open(path).await?;
    let mut data = vec![];
    file.read_to_end(&mut data).await?;
    Ok(data)
}

pub async fn load_certificates(file: &str) -> io::Result<Vec<x509_cert::Certificate>> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(file);
    let url =
        Url::from_file_path(&path).map_err(|_| io::Error::new(ErrorKind::Other, "invalid path"))?;
    let client = x509_client::X509Client::<DefaultX509Iterator>::new(X509ClientConfiguration {
        strict: false,
        files: true,
        limit: None,
        http_client: None,
    });
    Ok(client
        .get_all(&url)
        .await
        .map_err(|e| io::Error::new(ErrorKind::Other, e.to_string()))?
        .into_iter()
        .collect())
}
