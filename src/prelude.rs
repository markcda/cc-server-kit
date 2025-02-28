//! Standard prelude to import needed tools at once.

#[cfg(feature = "cc-utils")]
pub use cc_utils::{
  self,
  prelude::{Consider, ErrorResponse, Json, MResult, MsgPack, MsgPackParser, OK, json, msgpack, ok},
};

pub use crate::generic_setup::{GenericSetup, GenericValues, load_generic_config, load_generic_state};
pub use crate::startup::{get_root_router, start};
pub use salvo;
pub use tracing;
pub use tracing::instrument;

pub use salvo::handler;
pub use salvo::{Depot, Request, Router};

#[cfg(feature = "oapi")]
pub use salvo::oapi::endpoint;

#[cfg(feature = "test")]
pub use crate::test_exts::*;
