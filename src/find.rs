use crate::api::{CertificatePathValidation, PathValidator};
use crate::certificate::Certificate;
use crate::edge::{Edge, Edges};
use crate::report::{CertificateOrigin, Found, Report, ValidationFailure};
use crate::store::CertificateStore;
use crate::{X509PathFinderError, X509PathFinderResult};
use std::rc::Rc;
use std::time::{Duration, Instant};
use std::vec;
use url::Url;
use x509_client::provided::default::DefaultX509Iterator;
use x509_client::{X509Client, X509ClientConfiguration, X509ClientResult};

/// [`X509PathFinder`](crate::X509PathFinder) configuration
pub struct X509PathFinderConfiguration<V>
where
    V: PathValidator,
{
    /// limit runtime of path search. Actual limit will be N * HTTP timeout. See `Reqwest` docs for setting HTTP connection timeout.
    pub limit: Duration,
    /// Optional client to find additional certificates by parsing URLs from [Authority Information Access](https://datatracker.ietf.org/doc/html/rfc5280#section-4.2.2.1) extensions
    pub aia: Option<X509ClientConfiguration>,
    /// [`PathValidator`](crate::api::PathValidator) implementation
    pub validator: V,
    /// Bridge and cross signed-certificates to use for path finding
    pub certificates: Vec<crate::Certificate>,
}

/// X509 Path Finder
pub struct X509PathFinder<V>
where
    V: PathValidator,
{
    limit: Duration,
    aia: Option<X509Client<DefaultX509Iterator>>,
    validator: V,
    store: CertificateStore,
    edges: Edges,
}

impl<V> X509PathFinder<V>
where
    V: PathValidator,
    X509PathFinderError: From<<V as PathValidator>::PathValidatorError>,
{
    /// Instantiate new X509PathFinder with configuration
    pub fn new(config: X509PathFinderConfiguration<V>) -> Self
    where
        X509PathFinderError: From<der::Error>,
    {
        X509PathFinder {
            limit: config.limit,
            aia: config.aia.map(X509Client::new),
            validator: config.validator,
            store: CertificateStore::from_iter(config.certificates.into_iter().map(|c| c.into())),
            edges: Edges::new(),
        }
    }

    /// Find certificate path, returning [`Report`](crate::report::Report)
    pub async fn find(mut self, target: crate::Certificate) -> X509PathFinderResult<Report> {
        self.edges.start(target.into());
        let start = Instant::now();
        let mut failures = vec![];

        while let Some(edge) = self.edges.next() {
            if self.limit != Duration::ZERO && Instant::now() - start > self.limit {
                return Err(X509PathFinderError::Error("limit exceeded".to_string()));
            }

            if edge.as_ref() == &Edge::End {
                let (path, origin) = self.edges.path(&edge);
                match self
                    .validator
                    .validate(path.iter().map(|c| c.inner()).collect())?
                {
                    CertificatePathValidation::Found => {
                        drop(self);
                        let path: Vec<crate::Certificate> = path
                            .into_iter()
                            .map(|c| {
                                Rc::into_inner(c)
                                    .expect("all should be dropped")
                                    .into_inner()
                            })
                            .collect();

                        return Ok(Report {
                            found: Some(Found { path, origin }),
                            duration: Instant::now() - start,
                            failures,
                        });
                    }
                    CertificatePathValidation::NotFound(reason) => {
                        failures.push(ValidationFailure { origin, reason });
                    }
                }
            }

            if self.edges.visited(edge.as_ref()) {
                continue;
            }

            self.edges.visit(edge.clone());

            self.next(edge).await?;
        }

        Ok(Report {
            found: None,
            duration: Instant::now() - start,
            failures,
        })
    }

    async fn next(&mut self, edge: Rc<Edge>) -> X509PathFinderResult<()> {
        match edge.as_ref() {
            // edge is leaf certificate, search for issuer candidates
            Edge::Certificate(edge_certificate, _) => {
                let mut store_candidates = self.next_store(edge_certificate.clone());

                // return issuer candidates from store or try aia
                if !store_candidates.is_empty() {
                    // append any aia edges
                    let aia_urls = edge_certificate.aia();
                    let mut aia_edges = aia_urls
                        .iter()
                        .map(|u| Edge::Url(u.clone(), edge_certificate.clone()).into())
                        .collect::<Vec<Rc<Edge>>>();
                    store_candidates.append(&mut aia_edges);
                    self.edges.extend(edge.clone(), store_candidates);
                    Ok(())
                } else {
                    self.edges
                        .extend(edge.clone(), self.next_aia(edge_certificate.clone()));
                    Ok(())
                }
            }
            // edge is url, download certificates, search for issuer candidates
            Edge::Url(url, edge_certificate) => {
                let url_edges = self.next_url(edge_certificate.as_ref(), url).await;
                self.edges.extend(edge, url_edges);
                Ok(())
            }
            // edge is end, stop search
            Edge::End => Ok(()),
        }
    }

    // return non self-signed issuer candidates from store
    fn next_store(&self, parent_certificate: Rc<Certificate>) -> Vec<Rc<Edge>> {
        self.store
            .issuers(parent_certificate.as_ref())
            .into_iter()
            .filter_map(|candidate| {
                // filter out self-signed
                if !candidate.issued(candidate.as_ref()) {
                    Some(Edge::Certificate(candidate.clone(), CertificateOrigin::Store).into())
                } else {
                    None
                }
            })
            .collect()
    }

    // download certificates, insert into store, return non self-signed issuer candidates
    async fn next_url(&mut self, parent_certificate: &Certificate, url: &Url) -> Vec<Rc<Edge>> {
        let candidates = self
            .get_all(url)
            .await
            .unwrap_or_else(|_| vec![])
            .into_iter()
            .filter_map(|candidate| {
                let candidate = self.store.insert(candidate);
                // filtering out self-signed
                if candidate.issued(candidate.as_ref()) {
                    None
                } else if candidate.issued(parent_certificate) {
                    // url is issuer, return as certificate edge
                    Some(
                        Edge::Certificate(candidate.clone(), CertificateOrigin::Url(url.clone()))
                            .into(),
                    )
                } else {
                    None
                }
            })
            .collect::<Vec<Rc<Edge>>>();

        // no issuer candidates, return end edge
        if candidates.is_empty() {
            vec![Edge::End.into()]
        } else {
            candidates
        }
    }

    // if aia enabled, return aia edges
    fn next_aia(&self, parent_certificate: Rc<Certificate>) -> Vec<Rc<Edge>> {
        // aia disabled, return end edge
        if self.aia.is_none() {
            return vec![Edge::End.into()];
        }

        let aia_urls = parent_certificate.aia();

        // no aia urls found, return end edge
        if aia_urls.is_empty() {
            return vec![Edge::End.into()];
        }

        aia_urls
            .iter()
            .map(|u| Edge::Url(u.clone(), parent_certificate.clone()).into())
            .collect()
    }

    async fn get_all(&self, url: &Url) -> X509ClientResult<Vec<Certificate>> {
        if let Some(client) = &self.aia {
            Ok(client
                .get_all(url)
                .await?
                .into_iter()
                .map(|c| c.into())
                .collect())
        } else {
            Ok(vec![])
        }
    }
}
