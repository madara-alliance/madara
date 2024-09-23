use std::{borrow::Cow, collections::HashMap};

use mp_block::{BlockId, BlockTag};
use reqwest::{
    header::{HeaderMap, HeaderName, HeaderValue},
    Client,
};
use serde::de::DeserializeOwned;
use starknet_types_core::felt::Felt;
use url::Url;

use crate::error::{SequencerError, StarknetError};

#[derive(Debug, Clone)]
pub struct RequestBuilder<'a> {
    client: &'a Client,
    url: Url,
    params: HashMap<Cow<'static, str>, String>,
    headers: HeaderMap,
}

impl<'a> RequestBuilder<'a> {
    pub fn new(client: &'a Client, base_url: Url) -> Self {
        Self { client, url: base_url, params: HashMap::new(), headers: HeaderMap::new() }
    }

    pub fn add_uri_segment(mut self, segment: &str) -> Result<Self, url::ParseError> {
        self.url = self.url.join(segment)?;
        Ok(self)
    }

    pub fn add_header(mut self, name: HeaderName, value: HeaderValue) -> Self {
        self.headers.insert(name, value);
        self
    }

    pub fn add_param(mut self, name: Cow<'static, str>, value: &str) -> Self {
        self.params.insert(name, value.to_string());
        self
    }

    pub fn with_block_id(mut self, block_id: BlockId) -> Self {
        match block_id {
            BlockId::Hash(hash) => {
                self = self.add_param(Cow::from("blockHash"), &format!("0x{:x}", hash));
            }
            BlockId::Number(number) => {
                self = self.add_param(Cow::from("blockNumber"), &number.to_string());
            }
            BlockId::Tag(tag) => {
                let tag = match tag {
                    BlockTag::Latest => "latest",
                    BlockTag::Pending => "pending",
                };
                self = self.add_param(Cow::from("blockNumber"), tag);
            }
        }
        self
    }

    pub fn with_class_hash(mut self, class_hash: Felt) -> Self {
        self = self.add_param(Cow::from("classHash"), &format!("0x{:x}", class_hash));
        self
    }

    pub async fn send_get<T>(self) -> Result<T, SequencerError>
    where
        T: DeserializeOwned,
    {
        let mut request = self.client.get(self.url);

        for (key, value) in self.headers.iter() {
            request = request.header(key, value);
        }

        let response = request.query(&self.params).send().await?;

        unpack(response).await
    }

    pub async fn send_post<T>(self) -> Result<T, SequencerError>
    where
        T: DeserializeOwned,
    {
        let mut request = self.client.post(self.url);

        for (key, value) in self.headers.iter() {
            request = request.header(key, value);
        }

        let response = request.form(&self.params).send().await?;
        Ok(response.json().await?)
    }
}

async fn unpack<T>(response: reqwest::Response) -> Result<T, SequencerError>
where
    T: ::serde::de::DeserializeOwned,
{
    let status = response.status();
    if status == reqwest::StatusCode::INTERNAL_SERVER_ERROR || status == reqwest::StatusCode::BAD_REQUEST {
        let error = match response.json::<StarknetError>().await {
            Ok(e) => SequencerError::StarknetError(e),
            Err(e) if e.is_decode() => SequencerError::InvalidStarknetErrorVariant,
            Err(e) => SequencerError::ReqwestError(e),
        };
        return Err(error);
    }

    Ok(response.json::<T>().await?)
}
