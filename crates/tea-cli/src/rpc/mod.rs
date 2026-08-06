//! Strict bounded LF-delimited RPC mode.

mod reader;
mod server;
mod types;
mod writer;

pub use reader::{MAX_RPC_FRAME_BYTES, RpcFrameReader, RpcReadError};
pub use server::{MAX_RPC_IN_FLIGHT_REQUESTS, run, run_service};
pub use types::{
    RPC_VERSION, RpcError, RpcErrorCode, RpcOutput, RpcRequest, RpcRequestId, RpcRequestKind,
    RpcResponse,
};
pub use writer::{RPC_WRITER_DEADLINE, RPC_WRITER_QUEUE_CAPACITY, RpcLineWriter, RpcWriteError};
