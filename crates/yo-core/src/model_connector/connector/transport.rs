use reqwest::{Client, Url, redirect};

use super::{ConnectorError, ResponsesConnectorLimits, failure::configuration_failure};

pub(super) fn http_client(
    request_url: &Url,
    limits: &ResponsesConnectorLimits,
) -> Result<Client, ConnectorError> {
    let origin = Origin::from_url(request_url)?;
    let max_redirects = limits.max_redirects;
    let redirect_policy = redirect::Policy::custom(move |attempt| {
        match validate_redirect(
            &origin,
            attempt.url(),
            attempt.previous().len(),
            max_redirects,
        ) {
            Ok(()) => attempt.follow(),
            Err(message) => attempt.error(message),
        }
    });
    Client::builder()
        .connect_timeout(limits.connect_timeout)
        .redirect(redirect_policy)
        .retry(reqwest::retry::never())
        .build()
        .map_err(|_| configuration_failure("cannot initialize the model-connector HTTP client"))
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
