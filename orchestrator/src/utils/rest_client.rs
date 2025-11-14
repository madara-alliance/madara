use reqwest::{Client, Response, Result};
use serde::Serialize;
use url::Url;

pub struct RestClient {
    client: Client,
    base_url: Url,
}

impl RestClient {
    pub fn new(base_url: Url) -> Self {
        Self { client: Client::new(), base_url }
    }

    pub fn with_client(base_url: Url, client: Client) -> Self {
        Self { client, base_url }
    }

    fn url(&self, path: &str) -> Url {
        self.base_url.join(path).expect("Failed to join URL path")
    }

    pub async fn get(&self, path: &str) -> Result<Response> {
        self.client.get(self.url(path)).send().await
    }

    pub async fn post<T: Serialize>(&self, path: &str, body: &T) -> Result<Response> {
        self.client.post(self.url(path)).json(body).send().await
    }

    pub async fn put<T: Serialize>(&self, path: &str, body: &T) -> Result<Response> {
        self.client.put(self.url(path)).json(body).send().await
    }

    pub async fn patch<T: Serialize>(&self, path: &str, body: &T) -> Result<Response> {
        self.client.patch(self.url(path)).json(body).send().await
    }

    pub async fn delete(&self, path: &str) -> Result<Response> {
        self.client.delete(self.url(path)).send().await
    }

    pub async fn head(&self, path: &str) -> Result<Response> {
        self.client.head(self.url(path)).send().await
    }

    // For more control, expose the request builder
    pub fn request(&self, method: reqwest::Method, path: &str) -> Result<reqwest::RequestBuilder> {
        Ok(self.client.request(method, self.url(path)))
    }
}
