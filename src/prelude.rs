//! Standard prelude to import needed tools at once.

#[cfg(feature = "utils")]
pub use cc_utils::prelude::{MResult, ErrorResponse, Consider, ok, json, msgpack, OK, Json, MsgPack, MsgPackParser};

pub use salvo;
pub use tracing;
pub use tracing::instrument;
pub use crate::generic_setup::{
  GenericSetup,
  GenericValues,
  load_generic_config,
  load_generic_state,
};
pub use crate::startup::{
  get_root_router,
  start,
};

pub use salvo::handler;
pub use salvo::{Request, Depot, Router};

#[cfg(feature = "oapi")]
pub use salvo::oapi::endpoint;
