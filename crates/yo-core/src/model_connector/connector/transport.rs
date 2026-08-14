use reqwest::{Client, ClientBuilder, Url, redirect};

use super::{ConnectorError, ResponsesConnectorLimits, failure::configuration_failure};

pub(super) fn http_client(
    request_url: &Url,
    limits: &ResponsesConnectorLimits,
) -> Result<Client, ConnectorError> {
    http_client_builder(request_url, limits)?
        .build()
        .map_err(|_| configuration_failure("cannot initialize the model-connector HTTP client"))
}

#[cfg(test)]
pub(super) fn http_client_with_test_root(
    request_url: &Url,
    limits: &ResponsesConnectorLimits,
    root_pem: &[u8],
) -> Result<Client, ConnectorError> {
    let roots = reqwest::Certificate::from_pem_bundle(root_pem).map_err(|_| {
        configuration_failure("cannot parse the local-TLS fixture root certificate")
    })?;
    let [root] = roots.as_slice() else {
        return Err(configuration_failure(
            "the local-TLS fixture must provide exactly one root certificate",
        ));
    };
    http_client_builder(request_url, limits)?
        .add_root_certificate(root.clone())
        .build()
        .map_err(|_| {
            configuration_failure(
                "cannot initialize the model-connector HTTP client with the local-TLS fixture root",
            )
        })
}

fn http_client_builder(
    request_url: &Url,
    limits: &ResponsesConnectorLimits,
) -> Result<ClientBuilder, ConnectorError> {
    let _origin = Origin::from_url(request_url)?;
    Ok(Client::builder()
        .connect_timeout(limits.connect_timeout)
        .redirect(redirect::Policy::none())
        .retry(reqwest::retry::never()))
}

pub(super) fn validate_redirect(
    origin: &Origin,
    target: &Url,
    previous_count: usize,
    max_redirects: usize,
) -> Result<(), &'static str> {
    if previous_count >= max_redirects {
        Err("model-connector redirect limit exceeded")
    } else if !origin.matches(target) {
        Err("model-connector redirect changed origin")
    } else if !target.username().is_empty()
        || target.password().is_some()
        || target.query().is_some()
        || target.fragment().is_some()
    {
        Err("model-connector redirect target is not normalized")
    } else {
        Ok(())
    }
}

#[derive(Clone)]
pub(super) struct Origin {
    scheme: String,
    host: String,
    port: u16,
}

impl Origin {
    pub(super) fn from_url(url: &Url) -> Result<Self, ConnectorError> {
        let host = url
            .host_str()
            .ok_or_else(|| configuration_failure("model-connector request URL has no host"))?;
        let port = url.port_or_known_default().ok_or_else(|| {
            configuration_failure("model-connector request URL has no effective port")
        })?;
        Ok(Self {
            scheme: url.scheme().to_owned(),
            host: host.to_owned(),
            port,
        })
    }

    fn matches(&self, url: &Url) -> bool {
        url.scheme() == self.scheme
            && url.host_str() == Some(self.host.as_str())
            && url.port_or_known_default() == Some(self.port)
    }
}
