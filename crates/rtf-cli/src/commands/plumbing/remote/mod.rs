mod prepare;
mod request;

pub use prepare::{prepare_remote_trigger_payload, write_remote_trigger_payload_to_stdout};
pub use request::execute_remote_request;
