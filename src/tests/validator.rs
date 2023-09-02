use crate::api::{Certificate, CertificatePathValidation, PathValidator, PathValidatorError};
use crate::report::ValidateFailure;
use crate::X509PathFinderError;

pub struct TestPathValidator {
    roots: Vec<Certificate>,
}
impl TestPathValidator {
    pub fn new<I: Into<Certificate>>(roots: Vec<I>) -> Self {
        Self {
            roots: roots.into_iter().map(|c| c.into()).collect(),
        }
    }
}

impl PathValidator for TestPathValidator {
    type PathValidatorError = X509PathFinderError;

    fn validate(
        &self,
        path: Vec<Certificate>,
    ) -> Result<CertificatePathValidation, Self::PathValidatorError> {
        if path.is_empty() {
            return Ok(CertificatePathValidation::NotFound(ValidateFailure {
                path,
                reason: "path is empty".to_string(),
            }));
        }

        let ic = path.last().unwrap();

        for root in &self.roots {
            if root.issued(ic) {
                return Ok(CertificatePathValidation::Found(path));
            }
        }

        Ok(CertificatePathValidation::NotFound(ValidateFailure {
            path,
            reason: "could not find trusted path".to_string(),
        }))
    }
}

impl PathValidatorError for X509PathFinderError {}
