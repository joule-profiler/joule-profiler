use std::collections::HashSet;

use serde::Deserialize;

#[derive(Debug, Default, Deserialize)]
pub struct RaplConfig {
    pub sockets_spec: Option<HashSet<u32>>,
}
