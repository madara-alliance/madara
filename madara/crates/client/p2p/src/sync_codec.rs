//! Part of this file is inspired by the wonderful pathfinder implementation

use async_trait::async_trait;
use futures::io::{AsyncReadExt, AsyncWriteExt};
use libp2p::futures::{AsyncRead, AsyncWrite};
use std::{io, marker::PhantomData};

pub mod protocols {
    //! This only handles 1 protocol version for now. In the future this file would need
    //! to be rewritten so that it handles returning responses for older protocol versions.

    macro_rules! define_protocols {
        { $( struct $type_name:ident = $name:literal ; )* } => {
            $(
                #[derive(Debug, Clone, Copy, Default)]
                pub struct $type_name;
                impl AsRef<str> for $type_name {
                    fn as_ref(&self) -> &str {
                        $name
                    }
                }
            )*
        }
    }

    define_protocols! {
        struct Headers = "/starknet/headers/0.1.0-rc.0";
        struct StateDiffs = "/starknet/state_diffs/0.1.0-rc.0";
        struct Classes = "/starknet/classes/0.1.0-rc.0";
        struct Transactions = "/starknet/transactions/0.1.0-rc.0";
        struct Events = "/starknet/events/0.1.0-rc.0";
    }
}

pub mod codecs {
    #![allow(clippy::identity_op)] // allow 1 * MiB
    #![allow(non_upper_case_globals)] // allow MiB name
    use super::*;

    const MiB: u64 = 1024 * 1024;

    pub type Headers =
        SyncCodec<protocols::Headers, mp_proto::model::BlockHeadersRequest, mp_proto::model::BlockHeadersResponse>;
    pub fn headers() -> Headers {
        SyncCodec::new(SyncCodecConfig { req_size_limit_bytes: 1 * MiB, res_size_limit_bytes: 1 * MiB })
    }
    pub type StateDiffs =
        SyncCodec<protocols::StateDiffs, mp_proto::model::StateDiffsRequest, mp_proto::model::StateDiffsResponse>;
    pub fn state_diffs() -> StateDiffs {
        SyncCodec::new(SyncCodecConfig { req_size_limit_bytes: 1 * MiB, res_size_limit_bytes: 1 * MiB })
    }
    pub type Classes = SyncCodec<protocols::Classes, mp_proto::model::ClassesRequest, mp_proto::model::ClassesResponse>;
    pub fn classes() -> Classes {
        SyncCodec::new(SyncCodecConfig { req_size_limit_bytes: 1 * MiB, res_size_limit_bytes: 4 * MiB })
    }
    pub type Transactions =
        SyncCodec<protocols::Transactions, mp_proto::model::TransactionsRequest, mp_proto::model::TransactionsResponse>;
    pub fn transactions() -> Transactions {
        SyncCodec::new(SyncCodecConfig { req_size_limit_bytes: 1 * MiB, res_size_limit_bytes: 1 * MiB })
    }
    pub type Events = SyncCodec<protocols::Events, mp_proto::model::EventsRequest, mp_proto::model::EventsResponse>;
    pub fn events() -> Events {
        SyncCodec::new(SyncCodecConfig { req_size_limit_bytes: 1 * MiB, res_size_limit_bytes: 1 * MiB })
    }
}

#[derive(Debug, Clone)]
pub struct SyncCodecConfig {
    pub req_size_limit_bytes: u64,
    pub res_size_limit_bytes: u64,
}

#[derive(Debug, Clone)]
pub struct SyncCodec<Protocol, Req, Res> {
    config: SyncCodecConfig,
    /// buffer reuse
    buf: Vec<u8>,
    _boo: PhantomData<(Protocol, Req, Res)>,
}

impl<Protocol, Req, Res> SyncCodec<Protocol, Req, Res> {
    pub fn new(config: SyncCodecConfig) -> Self {
        Self { buf: Vec::new(), config, _boo: PhantomData }
    }
}

#[async_trait]
impl<
        Protocol: AsRef<str> + Send + Clone,
        Req: prost::Message + Default + Send,
        Res: prost::Message + Default + Send,
    > p2p_stream::Codec for SyncCodec<Protocol, Req, Res>
{
    type Protocol = Protocol;
    type Request = Req;
    type Response = Res;

    async fn read_request<T>(&mut self, _protocol: &Protocol, io: &mut T) -> io::Result<Req>
    where
        T: AsyncRead + Unpin + Send,
    {
        self.buf.clear();
        io.take(self.config.req_size_limit_bytes).read_to_end(&mut self.buf).await?;
        Ok(Req::decode(self.buf.as_ref())?)
    }

    async fn read_response<T>(&mut self, _protocol: &Protocol, mut io: &mut T) -> io::Result<Res>
    where
        T: AsyncRead + Unpin + Send,
    {
        // Response is prepended with the message length
        // We do not directly use [`prost::Message::decode_length_delimited`] because we want to reject the message before reading it
        // if it's too long

        // unsigned_varint's error type implements Into and not From io::Error, so we have to map the error by hand
        let encoded_len = unsigned_varint::aio::read_usize(&mut io).await.map_err(Into::<io::Error>::into)?;
        if encoded_len > self.config.res_size_limit_bytes as _ {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "Response has length {} which exceeds the spec-defined limit of {}",
                    encoded_len, self.config.res_size_limit_bytes
                ),
            ));
        }

        self.buf.clear();
        self.buf.reserve(encoded_len);
        io.take(encoded_len as _).read_to_end(&mut self.buf).await?;
        if self.buf.len() != encoded_len {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }

        Ok(Res::decode(self.buf.as_ref())?)
    }

    async fn write_request<T>(&mut self, _protocol: &Protocol, io: &mut T, req: Req) -> io::Result<()>
    where
        T: AsyncWrite + Unpin + Send,
    {
        self.buf.clear();
        req.encode(&mut self.buf)?;
        io.write_all(&self.buf).await
    }

    async fn write_response<T>(&mut self, _protocol: &Protocol, io: &mut T, res: Res) -> io::Result<()>
    where
        T: AsyncWrite + Unpin + Send,
    {
        // we don't have to use unsigned_varint::aio::write_usize here we can just use prost's length delimited messages impl

        self.buf.clear();
        res.encode_length_delimited(&mut self.buf)?;
        io.write_all(&self.buf).await
    }
}
