use openssl::x509::store::X509StoreBuilder;
use openssl::x509::verify::X509VerifyFlags;
use openssl::x509::X509;
use std::sync::RwLock;
use std::time::Duration;
use x509_path_finder::provided::store::DefaultCertificateStore;
use x509_path_finder::provided::validator::openssl::OpenSSLPathValidator;
use x509_path_finder::report::CertificateOrigin;
use x509_path_finder::{X509PathFinder, X509PathFinderConfiguration};
use x509_path_finder_material::{load_certificates, load_material};

#[tokio::test]
async fn test_find() {
    let certificates = load_certificates("kim@id.vandelaybank.com-fullchain.pem")
        .await
        .unwrap();

    let root = load_material("vandelaybank.com.cer").await.unwrap();
    let root = X509::from_der(&root).unwrap();

    let mut builder = X509StoreBuilder::new().unwrap();
    builder.add_cert(root).unwrap();
    builder.set_flags(X509VerifyFlags::X509_STRICT).unwrap();
    let validator = OpenSSLPathValidator::new(builder.build());

    let store = DefaultCertificateStore::from_iter(certificates.clone());

    let mut search = X509PathFinder::new(X509PathFinderConfiguration {
        limit: Duration::default(),
        aia: None,
        store: RwLock::new(store).into(),
        validator,
    });

    let report = search.find(certificates[0].clone()).await.unwrap();

    let path = report.path.unwrap().into_iter();

    assert_eq!(2, path.len());
    assert_eq!(
        vec![CertificateOrigin::Find, CertificateOrigin::Store],
        report.origin.unwrap()
    );

    let report = search.find(certificates[1].clone()).await.unwrap();

    let path = report.path.unwrap().into_iter();

    assert_eq!(1, path.len());
    assert_eq!(vec![CertificateOrigin::Find], report.origin.unwrap());
}