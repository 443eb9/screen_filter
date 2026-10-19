use reqwest::{
    Method, Url,
    blocking::{ClientBuilder, Request},
    header::{HeaderName, HeaderValue},
};
use serde::Deserialize;
use std::time::Duration;

#[derive(Deserialize, Clone)]
pub struct Release {
    pub tag_name: String,
    pub html_url: String,
}

pub fn check_for_updates() -> Result<Option<Release>, Box<dyn std::error::Error>> {
    let client = ClientBuilder::new()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(15))
        .user_agent(
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:136.0) Gecko/20100101 Firefox/136.0",
        )
        .default_headers(
            [
                (
                    HeaderName::from_static("accept"),
                    HeaderValue::from_static("application/vnd.github+json"),
                ),
                (
                    HeaderName::from_static("x-github-api-version"),
                    HeaderValue::from_static("2022-11-28"),
                ),
            ]
            .into_iter()
            .collect(),
        )
        .build()?;

    let resp = client.execute(Request::new(
        Method::GET,
        Url::parse("https://api.github.com/repos/443eb9/screen_filter/releases")?,
    ))?;
    let releases = resp.error_for_status()?.json::<Vec<Release>>()?;
    if let Some(latest) = releases.first()
        && latest.tag_name.trim_start_matches('v') != crate::VERSION
    {
        return Ok(Some(latest.clone()));
    }

    Ok(None)
}
